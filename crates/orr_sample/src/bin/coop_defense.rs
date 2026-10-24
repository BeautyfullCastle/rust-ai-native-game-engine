fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{}", orr_sample::defense_app::HELP);
        return;
    }
    if let Err(error) =
        orr_sample::defense_app::Options::parse(args).and_then(orr_sample::defense_app::run)
    {
        eprintln!("Stormglass Foundry: {error}");
        std::process::exit(1);
    }
}
