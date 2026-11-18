//! Export a saved CollectDodgeV1 project with an explicitly trusted, already-built runtime.

const HELP: &str = "Usage: orr_export_collect --project DIR --runtime BINARY --runtime-sha256 SHA256 \\\n    --output NEW_DIR --trusted-runtime [--source-revision TOKEN]\n\n\
Creates a relocatable Linux x86-64 or Windows x86-64 CollectDodgeV1 folder. NEW_DIR must not exist.\n\
The runtime must already be built with the collect-dodge feature. No build, download,\n\
or package installation is performed. --trusted-runtime asserts that you know\n\
and trust the supplied executable; its hash and smoke run do not establish trust.\n\
--source-revision is optional declared provenance, not verified source identity.\n\
Run the exported run-collect-dodge launcher (run-collect-dodge.cmd on Windows, with a .exe runtime) from any working directory.";

fn main() {
    if let Err((code, error)) = run() {
        eprintln!("error: {error}");
        std::process::exit(code);
    }
}

fn run() -> Result<(), (i32, String)> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!("{HELP}");
        return Ok(());
    }
    #[cfg(any(all(target_os = "linux", target_arch = "x86_64"), all(windows, target_arch = "x86_64")))]
    {
        let options = parse(args).map_err(|error| (2, error))?;
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        let report = orr_sample::project_export::export_collect(&options).map_err(|error| (1, error))?;
        #[cfg(windows)]
        let report = orr_sample::windows_export::export_collect(&options).map_err(|error| (1, error))?;
        #[cfg(windows)]
        let shown = orr_sample::windows_path::display(&report.output);
        #[cfg(not(windows))]
        let shown = report.output.display().to_string();
        println!("export output: {shown}");
        println!("export project files: {}", report.project_files);
        println!("export content digest: {}", report.content_digest);
        Ok(())
    }
    #[cfg(not(any(all(target_os = "linux", target_arch = "x86_64"), all(windows, target_arch = "x86_64"))))]
    Err((1, "project-export supports only Linux x86-64 and Windows x86-64 hosts".into()))
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use orr_sample::project_export::ExportOptions;
#[cfg(all(windows, target_arch = "x86_64"))]
use orr_sample::windows_export::ExportOptions;
#[cfg(any(all(target_os = "linux", target_arch = "x86_64"), all(windows, target_arch = "x86_64")))]
use std::{collections::BTreeSet, ffi::OsString, path::PathBuf};

#[cfg(any(all(target_os = "linux", target_arch = "x86_64"), all(windows, target_arch = "x86_64")))]
fn parse(args: Vec<OsString>) -> Result<ExportOptions, String> {
    let mut project = None;
    let mut runtime = None;
    let mut runtime_sha256 = None;
    let mut output = None;
    let mut trusted_runtime = false;
    let mut source_revision = None;
    let mut seen = BTreeSet::new();
    let mut args = args.into_iter();
    while let Some(argument) = args.next() {
        let option = argument.to_str().ok_or("option names must be UTF-8")?;
        if !matches!(
            option,
            "--project"
                | "--runtime"
                | "--runtime-sha256"
                | "--output"
                | "--trusted-runtime"
                | "--source-revision"
        ) {
            return Err(format!("unknown option {option}; use --help for usage"));
        }
        if !seen.insert(option.to_owned()) {
            return Err(format!("{option} may be supplied only once"));
        }
        if option == "--trusted-runtime" {
            trusted_runtime = true;
            continue;
        }
        let value = args
            .next()
            .ok_or_else(|| format!("{option} requires a value"))?;
        if value.to_str().is_some_and(|value| value.starts_with("--")) {
            return Err(format!("{option} requires a value"));
        }
        match option {
            "--project" => project = Some(PathBuf::from(value)),
            "--runtime" => runtime = Some(PathBuf::from(value)),
            "--output" => output = Some(PathBuf::from(value)),
            "--runtime-sha256" => {
                runtime_sha256 = Some(value.into_string().map_err(|_| "SHA256 must be ASCII")?);
            }
            "--source-revision" => {
                source_revision = Some(
                    value
                        .into_string()
                        .map_err(|_| "source revision must be an ASCII token")?,
                );
            }
            _ => unreachable!("recognized value option"),
        }
    }
    if !trusted_runtime {
        return Err("--trusted-runtime is required: select a known trusted, already-built executable; a SHA256 match is not a trust assertion".into());
    }
    Ok(ExportOptions {
        project: project.ok_or("--project is required")?,
        runtime: runtime.ok_or("--runtime is required")?,
        runtime_sha256: runtime_sha256.ok_or("--runtime-sha256 is required")?,
        output: output.ok_or("--output is required")?,
        trusted_runtime,
        source_revision,
    })
}
