#[cfg(all(windows, feature = "project-create", feature = "room-project"))]
#[path = "../windows_room/cli.rs"]
mod windows_room_cli;

#[cfg(all(windows, feature = "project-create", feature = "room-project"))]
fn main() -> std::process::ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).take(5).collect();
    windows_room_cli::run("orr_new_room", args, false)
}

#[cfg(not(all(windows, feature = "project-create", feature = "room-project")))]
fn main() -> std::process::ExitCode {
    eprintln!(
        "orr_new_room requires Windows and --features project-create,room-project; no project was opened"
    );
    std::process::ExitCode::FAILURE
}
