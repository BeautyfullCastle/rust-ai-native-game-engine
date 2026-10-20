//! Read-only installed-package references in a saved Room project.
#![forbid(unsafe_code)]
#[cfg(feature = "room-project")]
#[path = "room_package_usage/authoring.rs"]
mod authoring;

#[cfg(feature = "room-project")]
fn main() -> std::process::ExitCode {
    let result = (|| -> Result<(), String> {
        let Some(request) = authoring::parse_args(std::env::args_os().skip(1))? else {
            println!("{}", authoring::USAGE);
            return Ok(());
        };
        let report = authoring::inspect_usage(&request)?;
        let text = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
        println!("{text}");
        Ok(())
    })();
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("room_package_usage: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(feature = "room-project"))]
fn main() -> std::process::ExitCode {
    eprintln!("room_package_usage requires --features room-project or room-animated-models; no project was opened");
    std::process::ExitCode::FAILURE
}

// Reuse the same cases without integration-test-driven sibling binary linking.
#[cfg(all(test, feature = "room-project"))]
#[path = "../tests/room_package_usage.rs"]
mod tests;
