//! Read-only authored NavigationYard3D player; no server or editor startup.
#[cfg(target_os = "linux")]
fn main() {
    match orr_navigation_view::player::parse(std::env::args_os().skip(1)) {
        Ok(None) => println!("{}", orr_navigation_view::player::USAGE),
        Err(error) => {
            eprintln!("{error}\n{}", orr_navigation_view::player::USAGE);
            std::process::exit(2);
        }
        Ok(Some(options)) => match orr_navigation_view::player::run(options) {
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
