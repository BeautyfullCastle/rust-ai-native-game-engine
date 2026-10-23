#[path = "authoring.rs"]
mod authoring;

use std::{ffi::OsString, process::ExitCode};

pub fn run(program_name: &str, args: Vec<OsString>, legacy_usage_exit: bool) -> ExitCode {
    let usage = authoring::USAGE.replacen("orr_new_room", program_name, 1);
    if args.len() == 1 && args[0] == "--help" {
        println!("{usage}");
        return ExitCode::SUCCESS;
    }
    let options = match authoring::Options::parse(args) {
        Ok(options) => options,
        Err(error) => {
            eprintln!(
                "{program_name}: {}",
                error.replace(authoring::USAGE, &usage)
            );
            return if legacy_usage_exit {
                ExitCode::FAILURE
            } else {
                ExitCode::from(2)
            };
        }
    };
    match authoring::create(&options) {
        Ok(report) => {
            println!("created project: {}", report.output.display());
            println!("template: room-escape-3d-v1");
            println!(
                "project initial checksum: 0x{:016x}",
                report.initial_checksum
            );
            for guid in report.entity_guids {
                println!("project entity: {guid}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{program_name}: {error}");
            ExitCode::FAILURE
        }
    }
}
