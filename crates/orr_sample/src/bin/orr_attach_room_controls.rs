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
        println!("orr_attach_room_controls --project DIR --controls NAME.json\nLinux x86_64 only. Plain schema-2 Room with omitted controls; no lighting, character or progress. Existing root-local controls only; no replacement. Close editors and other writers first. Backup: orr.project.before-controls.json (must not exist). Runtime/editor must support room-input-actions. After any failure inspect the manifest and backup before retrying.");
        return Ok(());
    }
    if args.len() != 4 || args[0] != "--project" || args[2] != "--controls" {
        return Err("expected --project DIR --controls NAME.json; use --help".into());
    }
    let name = args[3].to_str().ok_or("controls filename must be ASCII")?;
    let outcome = orr_sample::room_controls_attach::prepare(std::path::Path::new(&args[1]), name)?
        .commit()?;
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
