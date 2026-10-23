#![cfg(all(feature = "bundle", target_os = "linux", target_arch = "x86_64"))]
//! CPU-only negatives. Actual delivered native windows are a separate mandatory gate.
use orr_static_starter::{
    bundle::{export, Options, LAUNCHER},
    delivery::DeliveryGame,
};
use std::{
    fs,
    os::unix::fs::{symlink, PermissionsExt},
    process::Command,
};

#[test]
fn untrusted_runtime_is_not_executed() {
    let options = Options {
        runtime: "missing".into(),
        runtime_sha256: "0".repeat(64),
        scene: "missing".into(),
        output: "missing".into(),
        source_sha: "0".repeat(40),
        trusted_runtime: false,
    };
    assert!(export::<DeliveryGame>(&options)
        .unwrap_err()
        .contains("trusted"));
}

#[test]
fn launcher_rejects_overrides_unknowns_duplicates_and_relative_outputs() {
    let tmp = tempfile::tempdir().unwrap();
    let launcher = tmp.path().join("run-game");
    fs::write(&launcher, LAUNCHER).unwrap();
    fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755)).unwrap();
    for args in [
        vec!["--scene", "/tmp/scene"],
        vec!["delivery"],
        vec!["--game", "switchboard"],
        vec!["--"],
        vec!["--record"],
        vec!["--observe", "relative"],
        vec!["--record", "/a", "--replay", "/b"],
        vec!["--observe", "/a", "--observe", "/b"],
        vec!["--identity"],
    ] {
        let output = Command::new(&launcher).args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2));
    }
}

#[test]
fn launcher_relocates_with_spaces_and_preserves_exact_argument_boundaries() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("relocated bundle with spaces");
    fs::create_dir_all(root.join("bin")).unwrap();
    fs::create_dir(root.join("scene")).unwrap();
    fs::write(root.join("run-game"), LAUNCHER).unwrap();
    fs::write(
        root.join("bin/standalone"),
        "#!/bin/sh\nprintf '%s\\n' \"$@\"\n",
    )
    .unwrap();
    for p in ["run-game", "bin/standalone"] {
        fs::set_permissions(root.join(p), fs::Permissions::from_mode(0o755)).unwrap();
    }
    let empty = tmp.path().join("empty");
    fs::create_dir(&empty).unwrap();
    let output = Command::new(root.join("run-game"))
        .current_dir(empty)
        .args([
            "--record",
            "/evidence with spaces/record.orrp",
            "--observe",
            "/evidence with spaces/native.jsonl",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let expected = format!("--scene\n{}/scene/game.scene.yaml\n--record\n/evidence with spaces/record.orrp\n--observe\n/evidence with spaces/native.jsonl\n",root.display());
    assert_eq!(output.stdout, expected.as_bytes());
}

#[test]
fn linked_runtime_rejected_without_changing_input_or_output() {
    let tmp = tempfile::tempdir().unwrap();
    let real = tmp.path().join("real");
    fs::write(&real, b"original").unwrap();
    let link = tmp.path().join("runtime");
    symlink(&real, &link).unwrap();
    let options = Options {
        runtime: link,
        runtime_sha256: "0".repeat(64),
        scene: "unused".into(),
        output: tmp.path().join("output"),
        source_sha: "0".repeat(40),
        trusted_runtime: true,
    };
    assert!(export::<DeliveryGame>(&options).is_err());
    assert_eq!(fs::read(real).unwrap(), b"original");
    assert!(!options.output.exists());
}
