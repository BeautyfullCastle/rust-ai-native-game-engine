#[cfg(windows)]
#[path = "../source_authoring_cli.rs"]
mod source_authoring_cli;

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    let arguments: Vec<_> = std::env::args_os().skip(1).take(9001).collect();
    source_authoring_cli::run("orr_pkg_source", arguments)
}

#[cfg(not(windows))]
fn main() -> std::process::ExitCode {
    eprintln!("orr_pkg_source requires Windows; no source directory was opened");
    std::process::ExitCode::FAILURE
}
