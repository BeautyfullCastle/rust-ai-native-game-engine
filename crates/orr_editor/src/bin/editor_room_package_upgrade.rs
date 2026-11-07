fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && args[0] == "--help" {
        println!("{}", orr_editor::package_manager::referenced_copy::USAGE);
        return;
    }
    match orr_editor::package_manager::referenced_copy::run(args) {
        Ok(report) => println!(
            "{}",
            serde_json::to_string(&report).expect("serialize copy receipt")
        ),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
