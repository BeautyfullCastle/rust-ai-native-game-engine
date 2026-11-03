#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn main() {
    if let Err(error) = orr_editor::mesh_prop_delivery::export::run_cli(std::env::args_os().skip(1))
    {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn main() {
    eprintln!("Mesh prop headless delivery supports Linux x86_64 only");
    std::process::exit(1);
}
