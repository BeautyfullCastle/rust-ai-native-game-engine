//! Read-only authored NavigationYard3D player; no server or editor startup.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn main() {
    match orr_navigation_view::surface_player::parse_command(std::env::args_os().skip(1)) {
        Ok(None) => println!("{}", orr_navigation_view::surface_player::USAGE),
        Err(error) => {
            eprintln!("{error}\n{}", orr_navigation_view::surface_player::USAGE);
            std::process::exit(2);
        }
        Ok(Some(orr_navigation_view::surface_player::Command::Capabilities)) => {
            println!("{}", orr_navigation_view::surface_player::capabilities())
        }
        Ok(Some(command)) => match match command {
            orr_navigation_view::surface_player::Command::Scene(options) => {
                orr_navigation_view::surface_player::run(options)
            }
            orr_navigation_view::surface_player::Command::Bundle(options) => {
                orr_navigation_view::surface_player::run_bundle(options)
            }
            orr_navigation_view::surface_player::Command::Capabilities => unreachable!(),
        } {
            Ok(receipt) => println!("{receipt}"),
            Err(error) => {
                eprintln!("navigation player failed: {error}");
                std::process::exit(1);
            }
        },
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn main() {
    eprintln!("navigation_surface_player authored admission currently requires Linux");
    std::process::exit(1);
}
