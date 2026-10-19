//! Offline capped-cylinder static-model authoring; requires the existing import feature.
#[cfg(feature = "import")]
#[path = "static_model_cylinder/authoring.rs"]
mod authoring;

fn main() {
    #[cfg(feature = "import")]
    let result = authoring::run(std::env::args_os().skip(1));
    #[cfg(not(feature = "import"))]
    let result: Result<(), String> = Err("static_model_cylinder requires --features import".into());
    if let Err(error) = result {
        use std::io::Write;
        let _ = writeln!(std::io::stderr(), "error: {error}");
        std::process::exit(2);
    }
}
