use orr_package::{Project, Runtime};
use serde_json::{json, Value};
use std::{fs, path::Path, process::Command};

fn source(root: &Path, name: &str, deps: &[&str]) -> std::path::PathBuf {
    let dir = root.join(name);
    fs::create_dir(&dir).unwrap();
    let dependencies: std::collections::BTreeMap<_, _> =
        deps.iter().map(|name| (*name, "1.0.0")).collect();
    fs::write(
        dir.join("orr.package.json"),
        serde_json::to_vec(&json!({
            "schema":1,"name":name,"version":"1.0.0","engine":"^0.0.1",
            "capabilities":["sprite"],"dependencies":dependencies,"files":["asset.txt"]
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(dir.join("asset.txt"), name).unwrap();
    dir
}

#[test]
fn explain_cli_tracks_selection_removal_without_mutating_content() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let project_root = root.join("project");
    fs::create_dir(&project_root).unwrap();
    let leaf = source(&root, "leaf", &[]);
    let left = source(&root, "left", &["leaf"]);
    let right = source(&root, "right", &["leaf"]);
    let game = source(&root, "game", &["left", "right"]);
    let other = source(&root, "other", &["leaf"]);
    let project =
        Project::open_for_install(&project_root, Runtime::content_only().engine_version).unwrap();
    let lock = project
        .install_with_dependencies(&[game, other, leaf.clone()], &[right, left])
        .unwrap();
    let lock_path = project_root.join("orr.packages.lock.json");
    let before = fs::read(&lock_path).unwrap();
    let object = project_root
        .join(".orr/packages/objects")
        .join(&lock.packages["leaf"].digest)
        .join("asset.txt");
    let output = Command::new(env!("CARGO_BIN_EXE_orr_pkg"))
        .args(["explain", project_root.to_str().unwrap(), "leaf"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["direct"], true);
    assert_eq!(
        value["selected_by"],
        json!({"game":["game","left","leaf"], "leaf":["leaf"],"other":["other","leaf"]})
    );
    assert_eq!(value["required_by"], json!(["left", "other", "right"]));
    assert_eq!(value["capabilities"], json!(["sprite"]));
    assert_eq!(value["digest"], lock.packages["leaf"].digest);
    assert!(String::from_utf8_lossy(&output.stderr).contains("installed bytes were not verified"));
    assert_eq!(fs::read(&lock_path).unwrap(), before);
    assert_eq!(fs::read(&object).unwrap(), b"leaf");
    assert_eq!(fs::read(leaf.join("asset.txt")).unwrap(), b"leaf");
    assert!(!project_root.join(".orr/packages/writer.lock").exists());

    project.remove("leaf").unwrap();
    let explanation = project.explain("leaf").unwrap();
    assert!(!explanation.direct);
    assert_eq!(explanation.selected_by.len(), 2);
    project.remove("game").unwrap();
    assert_eq!(project.explain("leaf").unwrap().selected_by.len(), 1);
    project.remove("other").unwrap();
    assert!(project
        .explain("leaf")
        .unwrap_err()
        .to_string()
        .contains("missing package: leaf"));
    let missing = Command::new(env!("CARGO_BIN_EXE_orr_pkg"))
        .args(["explain", project_root.to_str().unwrap(), "leaf"])
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(1));
    assert!(missing.stdout.is_empty());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("missing package: leaf"));
    assert_eq!(fs::read(&object).unwrap(), b"leaf");
}

#[test]
fn explanation_preserves_validation_and_distinguishes_metadata_from_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let package = source(&root, "art", &[]);
    let project = Project::open_for_install(&root, Runtime::content_only().engine_version).unwrap();
    let lock = project.install(&[package]).unwrap();
    // Metadata tool cannot make a compiled capability promise on behalf of a host.
    let host = Project::open(&root, Runtime::content_only()).unwrap();
    assert!(host.explain("art").is_err());
    let object = root
        .join(".orr/packages/objects")
        .join(&lock.packages["art"].digest)
        .join("asset.txt");
    fs::write(&object, "tampered").unwrap();
    assert!(project.explain("art").is_ok());
    assert!(project.verify().is_err());
    let path = root.join("orr.packages.lock.json");
    let original = fs::read(&path).unwrap();
    let mut invalid: Value = serde_json::from_slice(&original).unwrap();
    invalid["direct"]["absent"] = json!("1.0.0");
    fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
    assert!(project.explain("art").is_err());
    invalid["schema"] = json!(999);
    fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
    assert!(project.explain("art").is_err());
}

#[test]
fn shortest_chain_beats_lexicographically_earlier_long_chain() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let leaf = source(&root, "leaf", &[]);
    let middle = source(&root, "middle", &["leaf"]);
    let early = source(&root, "aaa", &["middle"]);
    let late = source(&root, "zzz", &["leaf"]);
    let game = source(&root, "game", &["aaa", "zzz"]);
    let project = Project::open_for_install(&root, Runtime::content_only().engine_version).unwrap();
    project
        .install_with_dependencies(&[game], &[leaf, middle, early, late])
        .unwrap();
    let explanation = project.explain("leaf").unwrap();
    assert_eq!(explanation.selected_by["game"], ["game", "zzz", "leaf"]);
    assert_eq!(project.explain("leaf").unwrap(), explanation);
    assert!(project
        .explain("unknown")
        .unwrap_err()
        .to_string()
        .contains("missing package: unknown"));
}

#[test]
fn invalid_explanation_names_have_bounded_diagnostics() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let project = Project::open_for_install(&root, Runtime::content_only().engine_version).unwrap();
    for name in ["", "../secret", "line\nbreak", &"x".repeat(4096)] {
        assert_eq!(
            project.explain(name).unwrap_err().to_string(),
            "invalid package/capability name"
        );
    }
    assert!(!root.join("orr.packages.lock.json").exists());
    assert!(!root.join(".orr").exists());
}
