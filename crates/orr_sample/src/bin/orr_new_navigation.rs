//! Dedicated offline creator for the closed Terrain point-route playground.
use std::{collections::BTreeSet, path::PathBuf};
const HELP: &str = "orr_new_navigation --output ABSOLUTE_NEW_DIR --seed KEY\nCreates terrain-point-route-3d-v1 on Linux: an owned slope/hole terrain, pinned scene and closed project.\nKEY is 1..128 ASCII letters, digits, dots, underscores or hyphens. The output must not exist.\nNo package installation, downloads, builds or external scripts; one zero-clearance point agent.";
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(2);
    }
}
fn run() -> Result<(), String> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if arguments.len() == 1 && (arguments[0] == "--help" || arguments[0] == "-h") {
        println!("{HELP}");
        return Ok(());
    }
    let mut output = None;
    let mut seed = None;
    let mut seen = BTreeSet::new();
    let mut arguments = arguments.into_iter();
    while let Some(flag) = arguments.next() {
        let flag = flag.to_str().ok_or("option must be UTF-8")?;
        if !matches!(flag, "--output" | "--seed") || !seen.insert(flag.to_owned()) {
            return Err(format!("unknown or duplicate option {flag}; {HELP}"));
        }
        let value = arguments
            .next()
            .ok_or_else(|| format!("{flag} requires a value"))?;
        match flag {
            "--output" => output = Some(PathBuf::from(value)),
            "--seed" => seed = Some(value.into_string().map_err(|_| "seed must be ASCII")?),
            _ => unreachable!(),
        }
    }
    let output = output.ok_or("--output is required")?;
    let seed = seed.ok_or("--seed is required")?;
    #[cfg(target_os = "linux")]
    {
        let report =
            orr_sample::project_create::create(&orr_sample::project_create::CreateOptions {
                output,
                seed,
                template: orr_sample::project_create::NAVIGATION_TEMPLATE.into(),
            })?;
        println!(
            "created project: {}\ntemplate: {}\nproject initial checksum: 0x{:016x}",
            report.output.display(),
            report.template,
            report.initial_checksum
        );
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (output, seed);
        Err("project-create supports only Linux hosts".into())
    }
}
