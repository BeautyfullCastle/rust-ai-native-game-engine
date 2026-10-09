//! The actual bounded generator CLI, with new output paths and no copied fixture.
#![cfg(all(feature = "project-create", target_os = "linux"))]
#![allow(clippy::disallowed_types)]

use orr_sample::{
    project_create::{create, CreateOptions},
    project_runtime::PreparedRuntime,
};
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    process::{Command, Output},
};

fn command(cwd: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_orr_new_arena"));
    command
        .current_dir(cwd)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY");
    command
}
fn success(output: Output) {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn visit(root: &Path, path: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            assert!(!entry.file_type().unwrap().is_symlink());
            if entry.file_type().unwrap().is_dir() {
                visit(root, &entry.path(), files);
            } else {
                files.insert(
                    entry
                        .path()
                        .strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_owned(),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

#[test]
fn actual_cli_and_public_api_generate_identical_loadable_projects() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let cwd = root.join("empty cwd");
    fs::create_dir(&cwd).unwrap();
    let output = root.join("CLI project with spaces");
    success(
        command(&cwd)
            .arg("--output")
            .arg(&output)
            .args(["--template", "arena-2d-v1", "--seed", "cli-parity-seed"])
            .output()
            .unwrap(),
    );
    let report = create(&CreateOptions {
        output: root.join("API project"),
        template: "arena-2d-v1".into(),
        seed: "cli-parity-seed".into(),
    })
    .unwrap();
    assert_eq!(report.template, "arena-2d-v1");
    assert_eq!(report.seed, "cli-parity-seed");
    assert_eq!(report.entity_guids.len(), 2);
    assert_eq!(files(&output), files(&report.output));
    let runtime = PreparedRuntime::open(&output).unwrap();
    assert_eq!(runtime.initial_frame().checksum(), report.initial_checksum);
    for guid in &report.entity_guids {
        assert!(runtime.index().entity(guid).is_some());
    }
    let before = files(&output);
    let failed = command(&cwd)
        .arg("--output")
        .arg(&output)
        .args(["--template", "arena-2d-v1", "--seed", "overwrite-attempt"])
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert_eq!(
        files(&output),
        before,
        "CLI must never merge into or overwrite an existing project"
    );
    assert!(fs::read_dir(&cwd).unwrap().next().is_none());
}

#[test]
fn missing_duplicate_unknown_and_malformed_cli_arguments_do_not_publish() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let target = root.join("never published");
    let out = target.to_str().unwrap();
    for args in [
        vec![],
        vec!["--output"],
        vec!["--output", "--seed"],
        vec!["--unknown"],
        vec!["--output", out, "--output", out],
        vec!["--output", out, "--template", "arena-2d-v1"],
        vec!["--output", out, "--seed", "bounded"],
        vec![
            "--output",
            out,
            "--template",
            "arena-2d-v1",
            "--seed",
            "--fresh",
        ],
        vec![
            "--output",
            out,
            "--template",
            "arena-2d-v1",
            "--seed",
            "bounded",
            "--seed",
            "again",
        ],
        vec![
            "--output",
            out,
            "--template",
            "arena-2d-v1",
            "--seed",
            "bounded",
            "--template",
            "arena-2d-v1",
        ],
    ] {
        let result = command(&root).args(&args).output().unwrap();
        assert!(
            !result.status.success(),
            "malformed {args:?} unexpectedly succeeded"
        );
        assert!(!result.stderr.is_empty());
        assert!(!target.exists());
        assert!(
            fs::read_dir(&root).unwrap().next().is_none(),
            "failed CLI leaked output/staging files"
        );
    }
}

#[test]
fn invalid_seed_template_and_relative_destination_are_rejected_without_files() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let destination = root.join("never published");
    let huge = "x".repeat(4096);
    for (output, template, seed) in [
        (destination.as_path(), "unknown-template", "seed"),
        (destination.as_path(), "arena-2d-v1", ""),
        (destination.as_path(), "arena-2d-v1", "non-ascii-한글"),
        (destination.as_path(), "arena-2d-v1", "contains\nnewline"),
        (destination.as_path(), "arena-2d-v1", huge.as_str()),
        (Path::new("relative project"), "arena-2d-v1", "seed"),
    ] {
        let result = command(&root)
            .arg("--output")
            .arg(output)
            .args(["--template", template, "--seed", seed])
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(!result.stderr.is_empty());
        assert!(
            fs::read_dir(&root).unwrap().next().is_none(),
            "rejected input wrote to the filesystem"
        );
    }
}

#[cfg(feature = "collect-dodge")]
#[test]
fn collect_cli_requires_identity_and_matches_library() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let output = root.join("collect project");
    let base = [
        "--template",
        "collect-dodge-2d-v1",
        "--seed",
        "cli-parity-seed",
    ];
    let missing = command(&root)
        .arg("--output")
        .arg(&output)
        .args(base)
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert!(!output.exists());
    for id in ["not-uuid", "12345678-1234-1234-8234-123456789abc"] {
        assert!(!command(&root)
            .arg("--output")
            .arg(&output)
            .args(base)
            .args(["--game-id", id])
            .output()
            .unwrap()
            .status
            .success());
        assert!(!output.exists());
    }
    let id = "12345678-1234-4234-8234-123456789abc";
    success(
        command(&root)
            .arg("--output")
            .arg(&output)
            .args(base)
            .args(["--game-id", id])
            .output()
            .unwrap(),
    );
    let report = orr_sample::project_create::create_collect(
        &CreateOptions {
            output: root.join("api"),
            template: "collect-dodge-2d-v1".into(),
            seed: "cli-parity-seed".into(),
        },
        id,
    )
    .unwrap();
    assert_eq!(files(&output), files(&report.output));
    let before = files(&output);
    assert!(!command(&root)
        .arg("--output")
        .arg(&output)
        .args(base)
        .args(["--game-id", id])
        .output()
        .unwrap()
        .status
        .success());
    assert_eq!(files(&output), before);
}

#[test]
fn arena_rejects_collect_identity_option() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let output = root.join("arena");
    assert!(!command(&root)
        .arg("--output")
        .arg(&output)
        .args([
            "--template",
            "arena-2d-v1",
            "--seed",
            "seed",
            "--game-id",
            "12345678-1234-4234-8234-123456789abc"
        ])
        .output()
        .unwrap()
        .status
        .success());
    assert!(!output.exists());
}

#[cfg(not(feature = "collect-dodge"))]
#[test]
fn generator_without_collect_feature_rejects_collect_template() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let output = root.join("unsupported");
    let result = command(&root)
        .arg("--output")
        .arg(&output)
        .args([
            "--template",
            "collect-dodge-2d-v1",
            "--seed",
            "seed",
            "--game-id",
            "12345678-1234-4234-8234-123456789abc",
        ])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("requires collect-dodge feature"));
    assert!(!output.exists());
}
