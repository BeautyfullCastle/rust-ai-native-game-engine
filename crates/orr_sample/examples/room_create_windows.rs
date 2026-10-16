#[cfg(all(windows, feature = "project-create", feature = "room-project"))]
#[path = "room_create_windows/authoring.rs"]
mod authoring;

#[cfg(all(windows, feature = "project-create", feature = "room-project"))]
fn main() -> std::process::ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).take(5).collect();
    if args.len() == 1 && args[0] == "--help" {
        println!("{}", authoring::USAGE);
        return std::process::ExitCode::SUCCESS;
    }
    match authoring::Options::parse(args).and_then(|options| authoring::create(&options)) {
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
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("room_create_windows: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(all(windows, feature = "project-create", feature = "room-project")))]
fn main() -> std::process::ExitCode {
    eprintln!("room_create_windows requires Windows and --features project-create,room-project; no project was opened");
    std::process::ExitCode::FAILURE
}
