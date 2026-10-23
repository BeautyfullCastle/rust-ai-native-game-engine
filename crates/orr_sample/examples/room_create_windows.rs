#[cfg(all(windows, feature = "project-create", feature = "room-project"))]
#[path = "room_create_windows/authoring.rs"]
mod authoring;

#[cfg(all(windows, feature = "project-create", feature = "room-project"))]
fn main() -> std::process::ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).take(5).collect();
    authoring::run("room_create_windows", args, true)
}

#[cfg(not(all(windows, feature = "project-create", feature = "room-project")))]
fn main() -> std::process::ExitCode {
    eprintln!(
        "room_create_windows requires Windows and --features project-create,room-project; no project was opened"
    );
    std::process::ExitCode::FAILURE
}
