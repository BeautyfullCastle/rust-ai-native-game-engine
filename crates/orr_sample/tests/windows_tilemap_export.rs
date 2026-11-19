//! Real CLI export of a saved tilemap Arena project on Windows, through the
//! automatic-profile exporter (`orr_export_windows`). Not hostile-source isolation.
#![cfg(all(
    windows,
    target_arch = "x86_64",
    feature = "project-create",
    feature = "project-export",
    feature = "tilemap"
))]

use sha2::{Digest, Sha256};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn state(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|line| {
            line.starts_with("project initial checksum:") || line.starts_with("project tick:")
        })
        .collect()
}

fn run(runtime: &str, project: &Path, cwd: &Path) -> String {
    success(
        Command::new(runtime)
            .arg("--project")
            .arg(project)
            .args(["--headless", "--ticks", "120"])
            .current_dir(cwd)
            .output()
            .unwrap(),
    )
}

#[test]
fn windows_export_ships_the_tilemap_and_the_exported_game_matches_the_source_run() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("source project");
    success(
        Command::new(env!("CARGO_BIN_EXE_orr_new_arena"))
            .args([
                "--template",
                "arena-tilemap-2d-v1",
                "--seed",
                "tile-windows-export",
                "--output",
            ])
            .arg(&project)
            .output()
            .unwrap(),
    );
    let runtime_dir = temp.path().join("supplied runtime");
    fs::create_dir(&runtime_dir).unwrap();
    let runtime = runtime_dir.join("arena.exe");
    fs::copy(env!("CARGO_BIN_EXE_arena"), &runtime).unwrap();
    let hash: String = Sha256::digest(fs::read(&runtime).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let output = temp.path().join("export");
    success(
        Command::new(env!("CARGO_BIN_EXE_orr_export_windows"))
            .arg("--project")
            .arg(&project)
            .arg("--runtime")
            .arg(&runtime)
            .arg("--runtime-sha256")
            .arg(&hash)
            .arg("--trusted-runtime")
            .arg("--output")
            .arg(&output)
            .output()
            .unwrap(),
    );
    assert_eq!(
        fs::read(output.join("project").join("arena.tilemap.json")).unwrap(),
        fs::read(project.join("arena.tilemap.json")).unwrap()
    );
    let empty = temp.path().join("empty cwd");
    fs::create_dir(&empty).unwrap();
    let source = run(runtime.to_str().unwrap(), &project, &empty);
    let exported = run(
        output.join("bin").join("arena.exe").to_str().unwrap(),
        &output.join("project"),
        &empty,
    );
    assert_eq!(state(&source).len(), 2);
    assert_eq!(state(&source), state(&exported));
}

#[test]
fn a_missing_tilemap_sidecar_is_never_published() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("source project");
    success(
        Command::new(env!("CARGO_BIN_EXE_orr_new_arena"))
            .args([
                "--template",
                "arena-tilemap-2d-v1",
                "--seed",
                "tile-windows-missing",
                "--output",
            ])
            .arg(&project)
            .output()
            .unwrap(),
    );
    fs::remove_file(project.join("arena.tilemap.json")).unwrap();
    let runtime = temp.path().join("arena.exe");
    fs::copy(env!("CARGO_BIN_EXE_arena"), &runtime).unwrap();
    let hash: String = Sha256::digest(fs::read(&runtime).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let output = temp.path().join("rejected");
    let result = Command::new(env!("CARGO_BIN_EXE_orr_export_windows"))
        .arg("--project")
        .arg(&project)
        .arg("--runtime")
        .arg(&runtime)
        .arg("--runtime-sha256")
        .arg(&hash)
        .arg("--trusted-runtime")
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!output.exists());
}
