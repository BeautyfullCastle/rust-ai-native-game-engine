//! Decision 12, step 4: the terminal viewer reads the view stream and nothing else of Orrery.
//! It must not link the simulation, the session, the physics, the bridge, the view layer or a
//! game, directly or through another crate, and its sources must not name them.
#![allow(clippy::disallowed_types)]

use std::path::Path;
use std::process::Command;

/// The only crates of this workspace it may use: the format decoder.
const ALLOWED_ORR: &[&str] = &["orr_tui", "orr_viewstream"];

/// Crates of `[dependencies]` (names before `=`).
fn dependencies(manifest: &str) -> Vec<String> {
    let mut in_deps = false;
    let mut out = Vec::new();
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_deps = line == "[dependencies]" || line == "[build-dependencies]";
            continue;
        }
        if in_deps && !line.starts_with('#') {
            if let Some((name, _)) = line.split_once('=') {
                out.push(name.trim().to_string());
            }
        }
    }
    out
}

fn manifest() -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap()
}

#[test]
fn the_manifest_lists_no_simulation_crate() {
    let manifest = manifest();
    let deps = dependencies(&manifest);
    let orr: Vec<_> = deps.iter().filter(|d| d.starts_with("orr_")).collect();
    assert_eq!(orr, ["orr_viewstream"], "orr_tui may depend on the format decoder only: {deps:?}");
    // The decoder alone: `default-features = false` leaves out the producer side (which links the sim).
    let line = manifest.lines().find(|l| l.trim_start().starts_with("orr_viewstream") && !l.contains("[")).unwrap();
    assert!(line.contains("default-features = false"), "orr_viewstream must be used without its `producer` feature: {line}");
}

#[test]
fn the_whole_dependency_tree_has_no_simulation_crate() {
    // What cargo really builds for this package: every normal dependency, transitively, for all features of it.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = Command::new(env!("CARGO"))
        .args(["tree", "-p", "orr_tui", "-e", "normal", "--prefix", "none", "--locked", "--offline"])
        .current_dir(root)
        .output()
        .expect("run cargo tree");
    assert!(out.status.success(), "cargo tree failed: {}", String::from_utf8_lossy(&out.stderr));
    let tree = String::from_utf8_lossy(&out.stdout);
    let mut seen = 0;
    for line in tree.lines() {
        let name = line.split_whitespace().next().unwrap_or("");
        seen += 1;
        if name.starts_with("orr_") {
            assert!(ALLOWED_ORR.contains(&name), "orr_tui reaches {name} (only {ALLOWED_ORR:?} allowed):\n{tree}");
        }
    }
    assert!(seen > 5, "cargo tree printed too little:\n{tree}");
    for forbidden in ["tokio", "wgpu", "winit", "bytemuck"] {
        assert!(!tree.lines().any(|l| l.split_whitespace().next() == Some(forbidden)), "unexpected heavy dependency {forbidden}:\n{tree}");
    }
}

/// Whether `code` names the crate `name` itself. A longer identifier that starts with it is not the
/// crate: the C ABI function `orr_session_status` is not a use of `orr_session`.
fn names_crate(code: &str, name: &str) -> bool {
    let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    code.match_indices(name).any(|(at, _)| name.ends_with("::") || !code[at + name.len()..].chars().next().is_some_and(ident))
}

#[test]
fn the_sources_do_not_name_simulation_crates() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut checked = 0;
    for entry in std::fs::read_dir(src).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for (i, line) in text.lines().enumerate() {
            // Doc text may mention them ("does not link orr_sim"); code may not.
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            for forbidden in ["orr_ecs", "orr_sim", "orr_session", "orr_physics", "orr_bridge", "orr_view::", "orr_sample", "orr_ffi::", "orr_remote", "orr_testgame"] {
                assert!(!names_crate(code, forbidden), "{}:{} names {forbidden}: {line}", path.display(), i + 1);
            }
        }
        checked += 1;
    }
    assert!(checked >= 8);
}
