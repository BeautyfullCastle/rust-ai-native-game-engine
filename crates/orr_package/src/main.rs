use orr_package::{inspect_for_install, Project, Runtime};
use std::{env, path::PathBuf};
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || args[0] == "--help" {
        println!("orr_pkg <install PROJECT [--dry-run] --path SOURCE [--dependency-path SOURCE]... | remove PROJECT NAME [--dry-run] | restore PROJECT NAME DIGEST [--dry-run] | list PROJECT | explain PROJECT NAME | verify PROJECT | inspect SOURCE>\nOffline content only. Runtime capabilities must be validated by the consuming host.");
        return Ok(());
    }
    let engine = Runtime::content_only().engine_version;
    if args[0] == "inspect" && args.len() == 2 {
        println!(
            "{}",
            serde_json::to_string_pretty(&inspect_for_install(&args[1], engine)?)?
        );
    } else {
        let root = args.get(1).ok_or("project directory required")?;
        let project = Project::open_for_install(root, engine)?;
        if args[0] == "explain" && args.len() == 3 {
            println!(
                "{}",
                serde_json::to_string_pretty(&project.explain(&args[2])?)?
            );
            eprintln!("Lock metadata only; installed bytes were not verified. Compiled capability validation is deferred to the consuming host.");
            return Ok(());
        }
        let lock = match args[0].as_str() {
            "install" if args.len() >= 4 => {
                let mut sources = Vec::new();
                let mut candidates = Vec::new();
                let dry_run = args.get(2).is_some_and(|arg| arg == "--dry-run");
                let start = if dry_run { 3 } else { 2 };
                let mut rest = args[start..].chunks_exact(2);
                for pair in &mut rest {
                    match pair[0].as_str() {
                        "--path" => sources.push(PathBuf::from(&pair[1])),
                        "--dependency-path" => candidates.push(PathBuf::from(&pair[1])),
                        _ => return Err("install expects --path or --dependency-path".into()),
                    }
                }
                if !rest.remainder().is_empty() {
                    return Err("source path missing".into());
                }
                if dry_run {
                    println!("{}", serde_json::to_string_pretty(
                        &project.preview_install_with_dependencies(&sources, &candidates)?
                    )?);
                    preview_limits();
                    return Ok(());
                }
                project.install_with_dependencies(&sources, &candidates)?
            }
            "remove" if args.len() == 4 && args[3] == "--dry-run" => {
                println!("{}", serde_json::to_string_pretty(&project.preview_remove(&args[2])?)?);
                preview_limits();
                return Ok(());
            }
            "remove" if args.len() == 3 => project.remove(&args[2])?,
            "restore" if args.len() == 5 && args[4] == "--dry-run" => {
                println!("{}", serde_json::to_string_pretty(
                    &project.preview_restore_cached(&args[2], &args[3])?
                )?);
                preview_limits();
                return Ok(());
            }
            "restore" if args.len() == 4 => {
                project.restore_cached(&args[2], &args[3])?
            }
            "list" if args.len() == 2 => project.list()?,
            "verify" if args.len() == 2 => project.verify()?,
            _ => return Err("invalid command/arguments; use --help".into()),
        };
        println!("{}", serde_json::to_string_pretty(&lock)?);
    }
    eprintln!("Content/engine validation only; compiled capability validation is deferred to the consuming host.");
    Ok(())
}
fn preview_limits() {
    eprintln!("Read-only content/graph preview; no activation or writer reservation. Future write permissions, writer availability and inactive destination objects are not guaranteed. Compiled capability validation is deferred to the consuming host.");
}
fn main() {
    if let Err(e) = run() {
        eprintln!("orr_pkg: {e}");
        std::process::exit(1);
    }
}
