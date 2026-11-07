#![cfg(all(
    target_os = "linux",
    feature = "project-create",
    feature = "room-lighting"
))]
use std::{fs, process::Command};
fn run(args: &[&std::ffi::OsStr]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_orr_new_lit_room"))
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn actual_cli_creates_a_source_independent_admitted_lit_game_and_refuses_overwrite() {
    let t = tempfile::tempdir().unwrap();
    let source = t.path().join("light.json");
    fs::write(
        &source,
        orr_sample::room_lighting::Document::enabled_default()
            .to_bytes()
            .unwrap(),
    )
    .unwrap();
    let out = t.path().join("lit game");
    let args = [
        "--output".as_ref(),
        out.as_os_str(),
        "--seed".as_ref(),
        "cli-lit".as_ref(),
        "--lighting".as_ref(),
        source.as_os_str(),
    ];
    let result = run(&args);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let manifest = fs::read(out.join("orr.project.json")).unwrap();
    assert!(!run(&args).status.success());
    assert_eq!(fs::read(out.join("orr.project.json")).unwrap(), manifest);
    fs::remove_file(source).unwrap();
    let project =
        orr_sample::room_project::PreparedProject::open_with_lighting(&out, true).unwrap();
    assert!(project.lighting().unwrap().document.point_light.is_some());
    assert!(String::from_utf8(result.stdout)
        .unwrap()
        .contains("room-escape-lit-3d-v1"));
}
#[test]
fn malformed_cli_and_light_fail_before_destination_creation() {
    let t = tempfile::tempdir().unwrap();
    let out = t.path().join("never created");
    let source = t.path().join("invalid");
    fs::write(&source, b"invalid").unwrap();
    assert!(!run(&[
        "--output".as_ref(),
        out.as_os_str(),
        "--seed".as_ref(),
        "seed".as_ref(),
        "--lighting".as_ref(),
        source.as_os_str()
    ])
    .status
    .success());
    assert!(!out.exists());
    for args in [
        vec!["--seed", "x"],
        vec!["--output", "x", "--output", "y", "--lighting", "z"],
        vec!["--bogus", "x", "--seed", "y", "--lighting", "z"],
    ] {
        let args: Vec<_> = args.iter().map(|s| std::ffi::OsStr::new(*s)).collect();
        assert!(!run(&args).status.success());
    }
    assert!(run(&["--help".as_ref()]).status.success());
    assert!(!out.exists());
}
