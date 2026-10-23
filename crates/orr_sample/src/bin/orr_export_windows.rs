#[cfg(all(windows, target_arch = "x86_64", feature = "project-export"))]
fn main() -> std::process::ExitCode {
    use orr_sample::windows_export::{export, ExportOptions};
    use std::{collections::BTreeMap, ffi::OsString, path::PathBuf};

    const USAGE: &str = "orr_export_windows --project PATH --runtime TRUSTED.exe --runtime-sha256 HASH --output ABSOLUTE_NEW_DIRECTORY --trusted-runtime [--source-revision REVISION]";
    let arguments: Vec<OsString> = std::env::args_os().skip(1).take(17).collect();
    if arguments.len() == 1 && arguments[0] == "--help" {
        println!("{USAGE}");
        return std::process::ExitCode::SUCCESS;
    }
    let parsed = (|| -> Result<ExportOptions, String> {
        if arguments.len() > 16 {
            return Err("too many arguments".into());
        }
        let mut values = BTreeMap::new();
        let mut trusted_runtime = false;
        let mut arguments = arguments.into_iter();
        while let Some(flag) = arguments.next() {
            if flag == "--trusted-runtime" && !trusted_runtime {
                trusted_runtime = true;
                continue;
            }
            let flag = flag.into_string().map_err(|_| "option must be UTF-8")?;
            if !matches!(
                flag.as_str(),
                "--project" | "--runtime" | "--runtime-sha256" | "--output" | "--source-revision"
            ) || values.contains_key(&flag)
            {
                return Err(format!("unknown or duplicate option: {flag}"));
            }
            let value = arguments
                .next()
                .ok_or_else(|| format!("{flag} requires a value"))?;
            if value.is_empty() || value.as_encoded_bytes().len() > 4096 {
                return Err(format!("{flag} requires 1..=4096 native bytes"));
            }
            values.insert(flag, value);
        }
        let mut take = |name: &str| {
            values
                .remove(name)
                .ok_or_else(|| format!("{name} is required"))
        };
        let project = PathBuf::from(take("--project")?);
        let runtime = PathBuf::from(take("--runtime")?);
        let runtime_sha256 = take("--runtime-sha256")?
            .into_string()
            .map_err(|_| "runtime SHA-256 must be UTF-8")?;
        let output = PathBuf::from(take("--output")?);
        let source_revision = values
            .remove("--source-revision")
            .map(|value| {
                value
                    .into_string()
                    .map_err(|_| "source revision must be UTF-8")
            })
            .transpose()?;
        Ok(ExportOptions {
            project,
            runtime,
            runtime_sha256,
            output,
            trusted_runtime,
            source_revision,
        })
    })();
    let options = match parsed {
        Ok(options) => options,
        Err(error) => {
            eprintln!("orr_export_windows: {error}\n{USAGE}");
            return std::process::ExitCode::from(2);
        }
    };
    match export(&options) {
        Ok(report) => {
            println!("exported: {}", report.output.display());
            println!("content digest: {}", report.content_digest);
            println!("project files: {}", report.project_files);
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("orr_export_windows: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(all(windows, target_arch = "x86_64", feature = "project-export")))]
fn main() -> std::process::ExitCode {
    eprintln!("orr_export_windows requires Windows x86-64 and --features project-export; no project was exported");
    std::process::ExitCode::FAILURE
}
