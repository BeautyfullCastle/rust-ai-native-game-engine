//! Decision 12, step 1: the editor sees the simulation only through the
//! bridge and ERP. It must not link the session, editing or physics crates
//! itself (they come in only through `orr_remote` and `orr_sample`, the
//! host's and the game's own crates), and its sources must not name them.
#![allow(clippy::disallowed_types)]

use std::path::Path;

/// The crates of `[dependencies]` in the manifest (names before `=`).
fn dependencies(manifest: &str) -> Vec<String> {
    let mut in_deps = false;
    let mut out = Vec::new();
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_deps = line == "[dependencies]";
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

/// The code of a line: no comment, no string literals (type names like `"orr_physics::Body"` are data).
fn code_of(line: &str) -> String {
    let mut out = String::new();
    let mut in_str = false;
    let mut prev = ' ';
    for c in line.chars() {
        if !in_str && c == '/' && prev == '/' {
            out.pop();
            break;
        }
        if c == '"' && prev != '\\' {
            in_str = !in_str;
        } else if !in_str {
            out.push(c);
        }
        prev = c;
    }
    out
}

/// Does `code` contain `word` as a whole identifier (`orr_edit` is not in `orr_editor`)?
fn names(code: &str, word: &str) -> bool {
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    code.match_indices(word).any(|(i, _)| {
        let before = code[..i].chars().next_back().is_none_or(|c| !is_ident(c));
        let after = code[i + word.len()..].chars().next().is_none_or(|c| !is_ident(c));
        before && (after || word.ends_with("::"))
    })
}

#[test]
fn the_editor_does_not_depend_on_the_simulation_crates() {
    let manifest = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    let deps = dependencies(&manifest);
    for forbidden in ["orr_session", "orr_edit", "orr_physics", "orr_sim", "orr_testgame", "orr_server", "orr_net"] {
        assert!(!deps.iter().any(|d| d == forbidden), "orr_editor must not depend on {forbidden}: {deps:?}");
    }
    // What it does use: the client side of the host, the bridge, the view and the game's view plugin.
    for wanted in ["orr_remote", "orr_bridge", "orr_view", "orr_render", "orr_rhi", "orr_reflect", "orr_fp", "orr_ecs", "orr_sample"] {
        assert!(deps.iter().any(|d| d == wanted), "expected {wanted} in {deps:?}");
    }
}

#[test]
fn the_editor_sources_do_not_name_the_simulation_crates() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut checked = 0;
    for entry in std::fs::read_dir(src).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for (i, line) in text.lines().enumerate() {
            let code = code_of(line);
            for forbidden in ["orr_session", "orr_edit", "orr_physics", "orr_sim::", "PlayController", "PlaySession", "EditorDoc"] {
                assert!(!names(&code, forbidden), "{}:{} names {forbidden}: {line}", path.display(), i + 1);
            }
        }
        checked += 1;
    }
    assert!(checked >= 10);
}

#[test]
fn terrain_authoring_stays_optional_and_simulation_integration_is_explicit() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(manifest.contains("terrain = [\"orr_model_bindings/terrain\", \"models\", \"dep:orr_terrain\", \"dep:orr_terrain_view\"]"));
    assert!(manifest.contains("default = []"));
    let view = std::fs::read_to_string(root.join("../orr_terrain_view/Cargo.toml")).unwrap();
    assert!(view.contains("default = []"));
    assert!(!manifest.contains("orr_terrain_view/gpu"));
    for name in ["orr_fp", "orr_ecs", "orr_sim", "orr_session", "orr_physics3d"] {
        let manifest = std::fs::read_to_string(root.join(format!("../{name}/Cargo.toml"))).unwrap();
        assert!(!dependencies(&manifest).iter().any(|d| d.starts_with("orr_terrain")), "{name} must remain independent of terrain authoring");
    }
    let games = std::fs::read_to_string(root.join("../orr_games/Cargo.toml")).unwrap();
    assert!(games.contains("terrain-physics ="));
    for line in games.lines().filter(|line| line.starts_with("orr_terrain")) {
        assert!(line.contains("optional = true"), "terrain simulation must remain opt-in: {line}");
    }
    assert!(manifest.lines().filter(|line| line.starts_with("orr_navigation =")).all(|line| line.contains("optional = true")));
}

#[test]
fn navigation_is_optional_and_independent_of_terrain_physics() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let editor = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(editor.contains("navigation = [\"terrain\", \"orr_remote/navigation\", \"dep:orr_navigation\"]"));
    assert!(editor.contains("default = []"));
    for name in ["orr_fp", "orr_ecs", "orr_sim", "orr_session", "orr_physics3d"] {
        let manifest = std::fs::read_to_string(root.join(format!("../{name}/Cargo.toml"))).unwrap();
        assert!(!dependencies(&manifest).iter().any(|d| d.starts_with("orr_navigation")), "{name} must stay navigation-free");
    }
    for name in ["orr_games", "orr_remote", "orr_editor"] {
        let manifest = std::fs::read_to_string(root.join(format!("../{name}/Cargo.toml"))).unwrap();
        let feature = manifest.lines().find(|line| line.starts_with("navigation =")).unwrap();
        for forbidden in ["terrain-physics", "animated-models", "irradiance-probes"] {
            assert!(!feature.contains(forbidden), "navigation must not require {forbidden}");
        }
        for line in manifest.lines().filter(|line| line.starts_with("orr_navigation")) {
            assert!(line.contains("optional = true"), "navigation must stay opt-in: {line}");
        }
    }
    let runtime = std::fs::read_to_string(root.join("../orr_navigation_runtime/Cargo.toml")).unwrap();
    for dep in dependencies(&runtime) {
        assert!(!["orr_bridge", "orr_sim", "orr_session", "orr_physics3d", "orr_render", "orr_remote"].contains(&dep.as_str()), "runtime dependency boundary: {dep}");
    }
}

#[test]
fn saved_projects_reuse_optional_sprite_packages_without_new_core_dependencies() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(manifest.contains("default = []"));
    assert!(manifest.contains("sprites = [\"dep:orr_sprite\", \"dep:orr_package\", \"dep:serde\", \"dep:tempfile\", \"orr_sample/project\"]"));
    for name in ["orr_sprite", "orr_package"] {
        let line = manifest.lines().find(|line| line.starts_with(&format!("{name} ="))).unwrap();
        assert!(line.contains("optional = true"), "{name} must remain optional");
    }
    let library = std::fs::read_to_string(root.join("src/lib.rs")).unwrap();
    assert!(library.contains("#[cfg(feature = \"sprites\")]\npub mod project;"));
    let package = std::fs::read_to_string(root.join("../orr_package/Cargo.toml")).unwrap();
    assert!(dependencies(&package).iter().all(|dependency| !dependency.starts_with("orr_")), "package metadata must stay independent of engine, simulation and renderer crates");
}
