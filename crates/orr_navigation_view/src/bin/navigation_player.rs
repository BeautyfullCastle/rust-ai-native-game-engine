//! Read-only authored NavigationYard3D player; no server or editor startup.
#[cfg(target_os = "linux")]
fn main() {
    match orr_navigation_view::player::parse_command(std::env::args_os().skip(1)) {
        Ok(None) => println!("{}", orr_navigation_view::player::USAGE),
        Err(error) => {
            eprintln!("{error}\n{}", orr_navigation_view::player::USAGE);
            std::process::exit(2);
        }
        Ok(Some(orr_navigation_view::player::Command::Capabilities)) => {
            println!("{}", orr_navigation_view::player::capabilities())
        }
        Ok(Some(command)) => match match command {
            orr_navigation_view::player::Command::Scene(options) => {
                orr_navigation_view::player::run(options)
            }
            orr_navigation_view::player::Command::Bundle(options) => {
                orr_navigation_view::player::run_bundle(options)
            }
            orr_navigation_view::player::Command::Capabilities => unreachable!(),
        } {
            Ok(receipt) => println!("{receipt}"),
            Err(error) => {
                eprintln!("navigation player failed: {error}");
                std::process::exit(1);
            }
        },
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("navigation_player authored admission currently requires Linux");
    std::process::exit(1);
}
