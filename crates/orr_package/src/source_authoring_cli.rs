#[path = "source_authoring.rs"]
mod authoring;

use std::ffi::OsString;

pub fn run(program_name: &str, arguments: Vec<OsString>) -> std::process::ExitCode {
    let usage = authoring::USAGE.replacen("package_source", program_name, 1);
    if arguments.len() == 1 && arguments[0] == "--help" {
        println!("{usage}");
        return std::process::ExitCode::SUCCESS;
    }
    let options = match authoring::Options::parse(arguments) {
        Ok(options) => options,
        Err(error) => {
            eprintln!(
                "{program_name}: {}",
                error.replace(authoring::USAGE, &usage)
            );
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
            let error = error.replace(authoring::USAGE, &usage);
            eprintln!("{program_name}: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
