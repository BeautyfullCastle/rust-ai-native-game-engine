const HELP:&str="orr_new_lit_room --output ABSENT_ABSOLUTE_PATH --seed ASCII_SEED --lighting FILE\nLinux; --features project-create,room-lighting. Creates a new complete Room with copied lighting; never edits an existing project.";
#[cfg(target_os = "linux")]
fn execute() -> Result<(), String> {
    use std::{collections::BTreeMap, ffi::OsString, path::PathBuf};
    let args: Vec<_> = std::env::args_os().skip(1).take(7).collect();
    if args.len() == 1 && args[0] == "--help" {
        println!("{HELP}");
        return Ok(());
    }
    if args.len() != 6 {
        return Err(HELP.into());
    }
    let mut values = BTreeMap::<String, OsString>::new();
    for pair in args.chunks(2) {
        let flag = pair[0].to_str().ok_or("invalid flag")?;
        if !["--output", "--seed", "--lighting"].contains(&flag)
            || pair[1].is_empty()
            || values.insert(flag.into(), pair[1].clone()).is_some()
        {
            return Err("unknown, duplicate or empty option".into());
        }
    }
    let seed = values["--seed"].to_str().ok_or("seed must be UTF-8")?;
    let light = orr_sample::room_lit_starter::read_lighting(&PathBuf::from(&values["--lighting"]))?;
    let report =
        orr_sample::room_lit_starter::create(&PathBuf::from(&values["--output"]), seed, &light)?;
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
