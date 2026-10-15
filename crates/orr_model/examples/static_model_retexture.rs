//! Offline cooked material texture authoring; requires the existing import feature.
#[cfg(feature = "import")]
#[path = "static_model_retexture/authoring.rs"]
mod authoring;

fn main() {
    #[cfg(feature = "import")]
    let result = authoring::run(std::env::args_os().skip(1));
    #[cfg(not(feature = "import"))]
    let result: Result<(), String> =
        Err("static_model_retexture requires --features import".into());
    if let Err(error) = result {
        use std::io::Write;
        let _ = writeln!(std::io::stderr(), "error: {error}");
        std::process::exit(2);
    }
}
