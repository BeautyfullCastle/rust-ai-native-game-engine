//! Offline authoring tool; requires the existing animation and import features.
#[cfg(all(feature = "animation", feature = "import"))]
#[path = "animation_clip_extract/authoring.rs"]
mod authoring;

fn main() {
    #[cfg(all(feature = "animation", feature = "import"))]
    let result = authoring::run(std::env::args_os().skip(1));
    #[cfg(not(all(feature = "animation", feature = "import")))]
    let result: Result<(), String> =
        Err("animation_clip_extract requires --features animation,import".into());
    if let Err(error) = result {
        use std::io::Write;
        let _ = writeln!(std::io::stderr(), "error: {error}");
        std::process::exit(2);
    }
}
