const HELP:&str="orr_new_lit_room --output ABSENT_ABSOLUTE_PATH --seed ASCII_SEED --lighting FILE [--character]\nLinux; --features project-create,room-lighting. --character additionally requires room-character. Creates a new complete Room with copied lighting; never edits an existing project.";
#[cfg(target_os = "linux")]
fn execute() -> Result<(), String> {
    use std::{collections::BTreeMap, ffi::OsString, path::PathBuf};
    let args: Vec<_> = std::env::args_os().skip(1).take(9).collect();
    if args.len() == 1 && args[0] == "--help" {
        println!("{HELP}");
        return Ok(());
    }
    if !(6..=7).contains(&args.len()) {
        return Err(HELP.into());
    }
    let mut values = BTreeMap::<String, OsString>::new();
    let mut character = false;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].to_str().ok_or("invalid flag")?;
        if flag == "--character" {
            if character {
                return Err("duplicate --character".into());
            }
            character = true;
            index += 1;
            continue;
        }
        if !["--output", "--seed", "--lighting"].contains(&flag) {
            return Err("unknown option".into());
        }
        let value = args.get(index + 1).ok_or("option requires a value")?;
        if value.is_empty()
            || value.to_str().is_some_and(|v| {
                ["--output", "--seed", "--lighting", "--character", "--help"].contains(&v)
            })
            || values.insert(flag.into(), value.clone()).is_some()
        {
            return Err("duplicate, missing or empty option value".into());
        }
        index += 2;
    }
    if values.len() != 3 {
        return Err("--output, --seed and --lighting are required".into());
    }
    if character && !cfg!(feature = "room-character") {
        return Err(
            "--character requires room-character feature; no input or project opened".into(),
        );
    }
    let seed = values["--seed"].to_str().ok_or("seed must be UTF-8")?;
    let light = orr_sample::room_lit_starter::read_lighting(&PathBuf::from(&values["--lighting"]))?;
    let report = orr_sample::room_lit_starter::create_with_character(
        &PathBuf::from(&values["--output"]),
        seed,
        &light,
        character,
    )?;
    println!(
        "created: {}\ntemplate: {}\ninitial checksum: 0x{:016x}",
        report.output.display(),
        report.template,
        report.initial_checksum
    );
    Ok(())
}
fn main() -> std::process::ExitCode {
    #[cfg(target_os = "linux")]
    match execute() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            std::process::ExitCode::FAILURE
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("Linux required; no project opened. {HELP}");
        std::process::ExitCode::FAILURE
    }
}
