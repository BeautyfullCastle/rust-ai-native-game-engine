fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(feature = "coop-defense-pixelfx")]
    if args == ["--pixelfx-licenses"] {
        print!("{}", orr_sample::defense_app::PIXELFX_LICENSES);
        return;
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{}", orr_sample::defense_app::HELP);
        #[cfg(feature = "coop-defense-pixelfx")]
        println!("Embedded pixelfx combat effects enabled. --pixelfx-licenses prints the bundled MIT license and notices without starting the game.");
        return;
    }
    if let Err(error) =
        orr_sample::defense_app::Options::parse(args).and_then(orr_sample::defense_app::run)
    {
        eprintln!("Stormglass Foundry: {error}");
        std::process::exit(1);
    }
}
