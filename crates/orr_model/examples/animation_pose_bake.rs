//! Offline sampled-pose authoring; requires existing animation and import features.
#[cfg(all(feature = "animation", feature = "import"))]
#[path = "animation_pose_bake/authoring.rs"]
mod authoring;

fn main() {
    #[cfg(all(feature = "animation", feature = "import"))]
    let result = authoring::run(std::env::args_os().skip(1));
    #[cfg(not(all(feature = "animation", feature = "import")))]
    let result: Result<(), String> =
        Err("animation_pose_bake requires --features animation,import".into());
    if let Err(error) = result {
        use std::io::Write;
        let _ = writeln!(std::io::stderr(), "error: {error}");
        std::process::exit(2);
    }
}
