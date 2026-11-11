//! Explicit trusted local embedded-game distribution; no implicit build/install.
fn main() {
    std::process::exit(i32::from(
        orr_package::stormglass_embedded_distribution::run_cli(std::env::args_os().skip(1)),
    ));
}
