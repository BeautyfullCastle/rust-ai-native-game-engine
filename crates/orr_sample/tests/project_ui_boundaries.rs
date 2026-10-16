//! Compile-time feature boundaries are a separate claim from UI rendering.
use std::{fs, path::Path};
#[test]
fn ui_metadata_and_core_remain_renderer_and_font_library_free() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for name in ["orr_package", "orr_fp", "orr_ecs", "orr_sim", "orr_session"] {
        let manifest = fs::read_to_string(root.join(format!("crates/{name}/Cargo.toml"))).unwrap();
        for forbidden in ["egui", "ttf-parser", "orr_editor", "orr_sample"] {
            assert!(!manifest.contains(forbidden), "{name} leaked {forbidden}");
        }
    }
    let sample = fs::read_to_string(root.join("crates/orr_sample/Cargo.toml")).unwrap();
    assert!(sample.contains("default = []"));
    let project = sample.lines().find(|l| l.starts_with("project =")).unwrap();
    assert!(!project.contains("game-ui") && !project.contains("egui") && !project.contains("ttf"));
    let editor = fs::read_to_string(root.join("crates/orr_editor/Cargo.toml")).unwrap();
    assert!(editor.contains("project-ui = [\"sprites\", \"orr_sample/game-ui\"]"));
    assert!(editor.contains("default = []"));
}
