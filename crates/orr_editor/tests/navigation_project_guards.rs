//! Explicit default-off project route and conflicting-authority checks.
use orr_editor::cli::Args;
fn parse(args: &[&str]) -> Result<Args, String> {
    Args::parse(args.iter().map(|s| s.to_string()))
}
#[test]
fn navigation_project_requires_explicit_feature_and_unique_authority() {
    let args = parse(&["--navigation-project", "authored-navigation"]);
    if cfg!(feature = "navigation-project") {
        assert_eq!(
            args.unwrap().navigation_project,
            Some("authored-navigation".into())
        );
    } else {
        assert!(args.unwrap_err().contains("navigation-project feature"));
    }
    for conflict in [
        vec!["--room-project", "room"],
        vec!["--collect-project", "collect"],
        vec!["--project", "arena"],
        vec!["--scene", "scene.yaml"],
        vec!["--game", "arena"],
        vec!["--connect", "ws://127.0.0.1:7777"],
        vec!["--script", "commands.txt"],
        vec!["--navigation-project", "other"],
    ] {
        let mut args = vec!["--navigation-project", "authored-navigation"];
        args.extend(conflict.clone());
        assert!(parse(&args).is_err());
        let mut args = conflict;
        args.extend(["--navigation-project", "authored-navigation"]);
        assert!(parse(&args).is_err());
    }
    assert!(parse(&["--navigation-project", ""]).is_err());
}
