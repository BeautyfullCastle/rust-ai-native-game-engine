fn main() {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    {
        eprintln!("Room controls attachment supports Linux x86_64 only");
        std::process::exit(1);
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && args[0] == "--help" {
        println!("orr_attach_room_controls --project DIR --controls NAME.json [--expect-controls OLD.json --backup VERSION.json]\nLinux x86_64 only. Plain schema-2 Room; no lighting, character or progress. Without both revision flags, requires omitted controls. Revision switches only the expected old reference to a distinct existing sidecar. Sidecars are never overwritten. Close editors and other writers first. First backup: orr.project.before-controls.json. Revision backup: explicit fresh root-local .json filename. Backups must not exist. Runtime/editor must support room-input-actions. After any failure inspect the manifest and backup before retrying.");
        return Ok(());
    }
    if !matches!(args.len(), 4 | 8)
        || args[0] != "--project"
        || args[2] != "--controls"
        || (args.len() == 8 && (args[4] != "--expect-controls" || args[6] != "--backup"))
    {
        return Err("expected --project DIR --controls NAME.json [--expect-controls OLD.json --backup VERSION.json]; use --help".into());
    }
    let name = args[3].to_str().ok_or("controls filename must be ASCII")?;
    let root = std::path::Path::new(&args[1]);
    let prepared = if args.len() == 8 {
        orr_sample::room_controls_attach::prepare_update(
            root,
            name,
            args[5]
                .to_str()
                .ok_or("old controls filename must be ASCII")?,
            args[7].to_str().ok_or("backup filename must be ASCII")?,
        )?
    } else {
        orr_sample::room_controls_attach::prepare(root, name)?
    };
    let outcome = prepared.commit()?;
    println!(
        "Attached {name}. Original manifest backup: {}",
        outcome.backup.display()
    );
    if let Some(warning) = outcome.durability_warning {
        eprintln!("APPLIED, but directory durability is uncertain: {warning}. Inspect before retrying; do not blindly roll back.");
        std::process::exit(3);
    }
    Ok(())
}
