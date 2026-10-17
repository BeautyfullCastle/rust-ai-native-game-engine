#[cfg(windows)]
#[path = "package_source/authoring.rs"]
mod authoring;

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    let arguments: Vec<_> = std::env::args_os().skip(1).take(9001).collect();
    authoring::run("package_source", arguments)
}

#[cfg(not(windows))]
fn main() -> std::process::ExitCode {
    eprintln!("package_source requires Windows; no source directory was opened");
    std::process::ExitCode::FAILURE
}
