use super::*;
use serde_json::json;
use std::{fs, os::unix::fs::symlink};
fn fixture() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir(t.path().join("build")).unwrap();
    for name in ["scene.yaml", "animations.json", "bank.json"] {
        fs::write(t.path().join(name), b"input").unwrap();
    }
    write(
        t.path(),
        &json!({"format":"orr_coop_project","version":1,"build":"build"}),
    );
    t
}
fn write(root: &Path, value: &Value) {
    fs::write(root.join("recipe.json"), serde_json::to_vec(value).unwrap()).unwrap();
}
fn load(root: &Path) -> Result<Recipe, String> {
    prepare(&root.join("recipe.json"), &root.join("out.html"))
}
#[test]
fn minimal_all_options_and_original_bytes_recheck() {
    let t = fixture();
    let root = t.path();
    let r = load(root).unwrap();
    assert_eq!(r.build_dir(), root.join("build"));
    assert!(r.scene().is_none());
    assert!(r.animations().is_none());
    assert!(r.audio().is_none());
    r.recheck().unwrap();
    write(
        root,
        &json!({"format":"orr_coop_project","version":1,"build":"build","scene":"scene.yaml","animations":"animations.json","audio":"bank.json"}),
    );
    assert!(r.recheck().is_err());
    let r = load(root).unwrap();
    assert_eq!(r.scene(), Some(root.join("scene.yaml").as_path()));
    assert_eq!(r.animations(), Some(root.join("animations.json").as_path()));
    assert_eq!(r.audio(), Some(root.join("bank.json").as_path()));
    assert!(!r.output().exists());
    r.recheck().unwrap();
    fs::remove_file(root.join("recipe.json")).unwrap();
    assert!(r.recheck().is_err());
}
#[test]
fn closed_schema_types_duplicates_and_bounds_fail_without_output() {
    let t = fixture();
    let root = t.path();
    for raw in [
        r#"{}"#,
        r#"[]"#,
        r#"{"format":"orr_coop_project","version":true,"build":"build"}"#,
        r#"{"format":"orr_coop_project","version":1.0,"build":"build"}"#,
        r#"{"format":"orr_coop_project","version":1,"build":"build","extra":0}"#,
        r#"{"format":"orr_coop_project","version":1,"build":"build","scene":null}"#,
        r#"{"format":"orr_coop_project","version":1,"build":"build","version":1}"#,
    ] {
        fs::write(root.join("recipe.json"), raw).unwrap();
        assert!(load(root).is_err(), "{raw}");
        assert!(!root.join("out.html").exists());
    }
    fs::write(root.join("recipe.json"), vec![b' '; 8193]).unwrap();
    assert!(load(root).is_err());
    fs::write(root.join("recipe.json"), [255]).unwrap();
    assert!(load(root).is_err());
    assert!(!root.join("out.html").exists());
}
#[test]
fn canonical_relative_paths_and_kind_must_match() {
    let t = fixture();
    let root = t.path();
    for path in [
        "",
        ".",
        "..",
        "./build",
        "../build",
        "build/",
        "build//x",
        "/build",
        "build\\x",
        "C:build",
        "build\0x",
        "scene.yaml",
    ] {
        write(
            root,
            &json!({"format":"orr_coop_project","version":1,"build":path}),
        );
        assert!(load(root).is_err(), "{path:?}");
    }
    for value in [json!(true), json!(1), json!(null), json!("x".repeat(1025))] {
        write(
            root,
            &json!({"format":"orr_coop_project","version":1,"build":value}),
        );
        assert!(load(root).is_err());
    }
    write(
        root,
        &json!({"format":"orr_coop_project","version":1,"build":"build","scene":"build"}),
    );
    assert!(load(root).is_err());
    assert!(!root.join("out.html").exists());
}
#[test]
fn symlink_recipe_directory_leaf_and_output_parent_refused() {
    let t = fixture();
    let root = t.path();
    symlink(root.join("build"), root.join("alias")).unwrap();
    write(
        root,
        &json!({"format":"orr_coop_project","version":1,"build":"alias"}),
    );
    assert!(load(root).is_err());
    symlink(root.join("scene.yaml"), root.join("scene-link")).unwrap();
    write(
        root,
        &json!({"format":"orr_coop_project","version":1,"build":"build","scene":"scene-link"}),
    );
    assert!(load(root).is_err());
    symlink(root, root.join("root-alias")).unwrap();
    write(
        root,
        &json!({"format":"orr_coop_project","version":1,"build":"build","scene":"root-alias/scene.yaml"}),
    );
    assert!(load(root).is_err());
    write(
        root,
        &json!({"format":"orr_coop_project","version":1,"build":"build"}),
    );
    assert!(prepare(&root.join("root-alias/recipe.json"), &root.join("out.html")).is_err());
    symlink(root.join("recipe.json"), root.join("recipe-link")).unwrap();
    assert!(prepare(&root.join("recipe-link"), &root.join("out.html")).is_err());
    assert!(prepare(&root.join("recipe.json"), &root.join("root-alias/out.html")).is_err());
    fs::write(root.join("out.html"), b"keep").unwrap();
    assert!(load(root).is_err());
    assert_eq!(fs::read(root.join("out.html")).unwrap(), b"keep");
}
#[test]
fn moved_tree_resolves_from_recipe_not_current_directory() {
    let outer = tempfile::tempdir().unwrap();
    let t = fixture();
    let moved = outer.path().join("relocated");
    fs::rename(t.path(), &moved).unwrap();
    let r = load(&moved).unwrap();
    assert_eq!(r.build_dir(), moved.join("build"));
    r.recheck().unwrap();
    assert!(!r.output().exists());
}
#[test]
fn explicit_flags_only_and_bounded_arguments() {
    let args = |values: &[&str]| values.iter().map(OsString::from).collect::<Vec<_>>();
    assert!(options(&args(&["--recipe", "recipe.json", "--output", "out.html"])).is_ok());
    for values in [
        vec!["recipe.json", "--output", "out.html"],
        vec!["--recipe", "a", "--recipe", "b"],
        vec!["--recipe", "a", "--other", "b"],
        vec!["--recipe", "", "--output", "b"],
    ] {
        assert!(options(&args(&values)).is_err());
    }
    assert!(options(&[
        "--recipe".into(),
        "a".repeat(4097).into(),
        "--output".into(),
        "b".into()
    ])
    .is_err());
}
