//! Create one offline, versioned Arena starter with an explicit authoring seed.
use std::{collections::BTreeMap, ffi::OsString, path::PathBuf};
const HELP: &str = "Usage: orr_new_arena --output ABSOLUTE_NEW_DIR --template arena-2d-v1 --seed KEY\n\nCreates one sprite-only two-player Arena starter on Linux. The parent must exist;\nall existing destinations are rejected. KEY is 1..128 ASCII letters, digits,\ndots, underscores or hyphens. Same seed/template/tool reproduces project bytes;\nchoose a different seed for fresh authored GUIDs. Runtime identity and shared\nplayer preferences do not change. No downloads, scripts or builds are executed.";
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
    let args = parse(args).map_err(|e| (2, e))?;
    #[cfg(target_os = "linux")]
    {
        let options = orr_sample::project_create::CreateOptions {
            output: PathBuf::from(&args["--output"]),
            template: args["--template"]
                .clone()
                .into_string()
                .map_err(|_| (2, "template must be UTF-8".into()))?,
            seed: args["--seed"]
                .clone()
                .into_string()
                .map_err(|_| (2, "seed must be ASCII".into()))?,
        };
        let report = orr_sample::project_create::create(&options).map_err(|e| (1, e))?;
        println!("created project: {}", report.output.display());
        println!("template: {}", report.template);
        println!("authoring seed: {}", report.seed);
        println!(
            "project initial checksum: 0x{:016x}",
            report.initial_checksum
        );
        for guid in report.entity_guids {
            println!("project entity: {guid}");
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (args, PathBuf::new());
        Err((1, "project-create supports only Linux hosts".into()))
    }
}
fn parse(args: Vec<OsString>) -> Result<BTreeMap<String, OsString>, String> {
    let mut options = BTreeMap::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let option = arg.to_str().ok_or("option names must be UTF-8")?;
        if !matches!(option, "--output" | "--template" | "--seed") {
            return Err(format!("unknown option {option}; use --help for usage"));
        }
        if options.contains_key(option) {
            return Err(format!("{option} may be supplied only once"));
        }
        let value = args
            .next()
            .ok_or_else(|| format!("{option} requires a value"))?;
        if value.to_str().is_some_and(|v| v.starts_with("--")) {
            return Err(format!("{option} requires a value"));
        }
        options.insert(option.into(), value);
    }
    for key in ["--output", "--template", "--seed"] {
        if !options.contains_key(key) {
            return Err(format!("{key} is required"));
        }
    }
    Ok(options)
}
