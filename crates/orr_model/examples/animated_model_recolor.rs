//! Offline animated material RGB authoring; features are always explicit.
fn main() {
    #[cfg(all(feature = "animation", feature = "import"))]
    let result = orr_model::animated_recolor::run(std::env::args_os().skip(1));
    #[cfg(not(all(feature = "animation", feature = "import")))]
    let result: Result<(), String> =
        Err("animated_model_recolor requires --features animation,import".into());
    if let Err(error) = result {
        use std::io::Write;
        let _ = writeln!(std::io::stderr(), "error: {error}");
        std::process::exit(2);
    }
}
