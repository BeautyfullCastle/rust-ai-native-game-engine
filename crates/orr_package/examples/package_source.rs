#[cfg(windows)]
#[path = "package_source/authoring.rs"]
mod authoring;

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    let arguments: Vec<_> = std::env::args_os().skip(1).take(9001).collect();
    if arguments.len() == 1 && arguments[0] == "--help" {
        println!("{}", authoring::USAGE);
        return std::process::ExitCode::SUCCESS;
    }
    let options = match authoring::Options::parse(arguments) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("package_source: {error}");
            return std::process::ExitCode::from(2);
        }
    };
    match authoring::create(&options) {
        Ok(report) => {
            println!("created manifest: {}", report.manifest.display());
            println!("declared files: {}", report.declared_files);
            println!("declared bytes: {}", report.declared_bytes);
            eprintln!("Source metadata/content checked only; dependency resolution and compiled capability validation remain with installation and the consuming host.");
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("package_source: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(windows))]
fn main() -> std::process::ExitCode {
    eprintln!("package_source requires Windows; no source directory was opened");
    std::process::ExitCode::FAILURE
}
