use std::{ffi::OsString, io, path::PathBuf, process::ExitCode};
const USAGE: &str = "Usage: orr_pkg_diff PROJECT PACKAGE --source DIRECTORY [--json]\nExit: 0 equal, 1 different, 2 error. Read-only; no install or dependency-resolution claim.\n";
fn run(args: Vec<OsString>) -> Result<u8, Box<dyn std::error::Error>> {
    if args.len() == 1 && args[0] == "--help" {
        orr_package_diff::emit(io::stdout().lock(), USAGE.as_bytes())?;
        return Ok(0);
    }
    if !(args.len() == 4 || args.len() == 5)
        || args[2] != "--source"
        || (args.len() == 5 && args[4] != "--json")
    {
        return Err(USAGE.into());
    }
    let package = args[1].to_str().ok_or("package name must be UTF-8")?;
    if package.starts_with('-') || args[0].is_empty() || args[3].is_empty() {
        return Err(USAGE.into());
    }
    let report =
        orr_package_diff::compare(&PathBuf::from(&args[0]), package, &PathBuf::from(&args[3]))?;
    let bytes = orr_package_diff::encode(&report, args.len() == 5)?;
    orr_package_diff::emit(io::stdout().lock(), &bytes)?;
    Ok(u8::from(report.different()))
}
fn main() -> ExitCode {
    match run(std::env::args_os().skip(1).collect()) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            // Best effort: stderr itself can be closed. Never panic on a broken pipe.
            use std::io::Write;
            let diagnostic: String = error.to_string().chars().take(4096).collect();
            let _ = writeln!(io::stderr().lock(), "orr_pkg_diff: {diagnostic}");
            ExitCode::from(2)
        }
    }
}
