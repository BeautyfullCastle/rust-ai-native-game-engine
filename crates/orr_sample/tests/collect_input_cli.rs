//! Executed CLI admission checks; no native window or physical input is claimed.
#![cfg(feature = "collect-dodge")]
#![allow(clippy::disallowed_types)]
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_collect_dodge"))
        .args(args)
        .output()
        .unwrap()
}
fn refused(args: &[&str], expected: &str) {
    let output = run(args);
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains(expected), "{error}");
    assert!(output.stdout.is_empty());
}
#[test]
fn help_describes_optional_map_and_semantic_headless_controls() {
    let output = run(&["--help"]);
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("--input-bindings FILE"));
    assert!(help.contains("--print-default-input-map"));
    assert!(help.contains("window-only"));
    assert!(help.contains("semantic"));
}
#[test]
fn map_flags_require_unambiguous_arguments() {
    refused(
        &["--project", "unused", "--input-bindings"],
        "requires FILE",
    );
    refused(
        &[
            "--project",
            "unused",
            "--input-bindings",
            "a",
            "--input-bindings",
            "b",
        ],
        "duplicate option",
    );
    refused(
        &["--print-default-input-map", "--project", "unused"],
        "must be used alone",
    );
    refused(
        &["--headless", "--print-default-input-map"],
        "must be used alone",
    );
}
#[test]
fn map_feature_and_headless_refusals_are_explicit() {
    let output = run(&["--print-default-input-map"]);
    if cfg!(feature = "input-actions") {
        assert!(output.status.success());
        #[cfg(feature = "input-actions")]
        {
            let map = orr_input::ActionMap::load(output.stdout.as_slice()).unwrap();
            assert_eq!(map, orr_sample::collect_app::input_actions::default_map());
        }
        refused(
            &[
                "--project",
                "unused",
                "--headless",
                "--ticks",
                "1",
                "--hold",
                "right",
                "--input-bindings",
                "unused.json",
            ],
            "window-only",
        );
    } else {
        assert!(!output.status.success());
        assert!(String::from_utf8(output.stderr)
            .unwrap()
            .contains("input-actions feature"));
        refused(
            &["--project", "unused", "--input-bindings", "unused.json"],
            "input-actions feature",
        );
    }
}
#[cfg(feature = "input-actions")]
#[test]
fn malformed_map_is_refused_before_missing_project_and_bytes_are_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let map = dir.path().join("controls.json");
    let bytes = br#"{"version":2,"actions":[]}"#;
    std::fs::write(&map, bytes).unwrap();
    refused(
        &[
            "--project",
            "missing-project",
            "--input-bindings",
            map.to_str().unwrap(),
        ],
        "unsupported input map version",
    );
    assert_eq!(std::fs::read(map).unwrap(), bytes);
}
#[cfg(feature = "input-actions")]
#[test]
fn valid_map_reaches_project_admission_without_writing_either_source() {
    let dir = tempfile::tempdir().unwrap();
    let map = dir.path().join("controls.json");
    orr_sample::collect_app::input_actions::default_map()
        .save_new(&map)
        .unwrap();
    let before = std::fs::read(&map).unwrap();
    // Deliberately malformed project: a valid map advances to project admission,
    // but no host, event loop, GPU or native window can be created.
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let manifest = project.join("orr.project.json");
    std::fs::write(&manifest, b"invalid project json").unwrap();
    let output = run(&[
        "--project",
        project.to_str().unwrap(),
        "--input-bindings",
        map.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(!error.contains("input map"), "{error}");
    assert!(!error.contains("event loop"), "{error}");
    assert_eq!(std::fs::read(map).unwrap(), before);
    assert_eq!(std::fs::read(manifest).unwrap(), b"invalid project json");
}
