#![cfg(feature = "navigation")]

use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_orr_mesh_cook")
}

fn fixture_source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/imported_scene_demo/foreground.glb")
        .canonicalize()
        .unwrap()
}

fn run(arguments: &[&str]) -> Output {
    Command::new(binary()).args(arguments).output().unwrap()
}

fn cook(source: &Path, output: &Path, extra: &[&str]) -> Output {
    let mut arguments = vec![
        "--source",
        source.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
    ];
    arguments.extend_from_slice(extra);
    run(&arguments)
}

#[test]
fn cooks_a_real_model_to_a_canonical_mesh_asset() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("foreground.GLB");
    let output = directory.path().join("foreground.ORRMESH");
    std::fs::copy(fixture_source(), &source).unwrap();
    let result = cook(&source, &output, &[]);

    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8(result.stdout).unwrap();
    assert!(stdout.contains("revision="));
    assert!(stdout.contains("triangles="));
    assert!(stdout.contains("bytes="));
    assert!(stdout.contains("point agents on open-sky triangles only"));
    assert!(std::fs::metadata(output).unwrap().len() > 48);
}

#[test]
fn refuses_to_clobber_an_existing_output() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("existing.orrmesh");
    let original = b"keep this output";
    std::fs::write(&output, original).unwrap();

    let result = cook(&fixture_source(), &output, &[]);

    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stderr).contains("refusing to overwrite"));
    assert_eq!(std::fs::read(output).unwrap(), original);
}

#[test]
fn rejects_invalid_arguments_before_creating_an_output() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let output = root.join("never-created.orrmesh");
    let source = fixture_source();
    let source_text = source.to_str().unwrap();
    let output_text = output.to_str().unwrap();
    let cases = [
        vec!["--source", "relative.glb", "--output", output_text],
        vec!["--source", "", "--output", output_text],
        vec!["--source", source_text, "--output", "relative.orrmesh"],
        vec!["--source", source_text, "--output", ""],
        vec![
            "--source",
            source_text,
            "--output",
            output_text,
            "--source",
            source_text,
        ],
        vec![
            "--source",
            source_text,
            "--output",
            output_text,
            "--mystery",
            "value",
        ],
        vec!["--source", source_text],
        vec!["--output", output_text],
        vec!["--source", source_text, "--output"],
        vec![
            "--source",
            source_text,
            "--output",
            output_text,
            "--output",
            output_text,
        ],
        vec![
            "--source",
            source_text,
            "--output",
            output_text,
            "--max-slope",
            "1",
            "--max-slope",
            "2",
        ],
        vec![
            "--source",
            source_text,
            "--output",
            output_text,
            "--max-slope",
            "4.0000152587890625",
        ],
        vec![
            "--source",
            source_text,
            "--output",
            output_text,
            "--max-slope",
            "not-a-number",
        ],
        vec![
            "--source",
            source_text,
            "--output",
            output_text,
            "--max-slope",
            "1",
            "--unknown",
            "value",
            "extra",
        ],
    ];

    for arguments in cases {
        let result = run(&arguments);
        assert_eq!(result.status.code(), Some(2), "args: {arguments:?}");
        assert!(!output.exists(), "args: {arguments:?}");
    }
}

#[test]
fn missing_source_does_not_create_an_output() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("missing.glb");
    let output = directory.path().join("never-created.orrmesh");
    let result = cook(&source, &output, &[]);

    assert_eq!(result.status.code(), Some(1));
    assert!(!output.exists());
}

#[test]
fn help_prints_usage_without_running_a_cook() {
    let result = run(&["--help"]);

    assert!(result.status.success());
    assert!(String::from_utf8_lossy(&result.stdout).contains("Usage: orr_mesh_cook"));
}
