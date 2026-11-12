//! Explicit trusted-runtime Navigation bundle exporter.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn main() {
    if let Err(error) = orr_navigation_view::bundle::export::run_cli(std::env::args_os().skip(1)) {
        eprintln!("navigation export failed: {error}");
        std::process::exit(1);
    }
}
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn main() {
    eprintln!("navigation_export requires Linux x86_64");
    std::process::exit(1);
}
