#![cfg(windows)]

use orr_package::{inspect_for_install, Project, Runtime};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

const MANIFEST: &str = "orr.package.json";

fn source_binary() -> &'static str {
    env!("CARGO_BIN_EXE_orr_pkg_source")
}

fn arguments(source: &Path, files: &[&str]) -> Vec<OsString> {
    let mut result = vec![
        "--source".into(),
        source.as_os_str().to_owned(),
        "--name".into(),
        "room-art".into(),
        "--version".into(),
        "1.0.0".into(),
        "--engine".into(),
        "^0.0.1".into(),
    ];
    for file in files {
        result.push("--file".into());
        result.push((*file).into());
    }
    result
}

fn run(arguments: &[OsString]) -> Output {
    Command::new(source_binary())
        .args(arguments)
        .output()
        .unwrap()
}

fn fixture(root: &Path, name: &str) -> PathBuf {
    let source = root.join(name);
    fs::create_dir_all(source.join("assets")).unwrap();
    fs::write(source.join("assets/room.glb"), b"room model bytes").unwrap();
    fs::write(source.join("LICENSE.txt"), b"license bytes").unwrap();
    fs::write(source.join("unlisted.txt"), b"not declared").unwrap();
    source
}

fn tree_snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn visit(root: &Path, current: &Path, result: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        let metadata = fs::symlink_metadata(current).unwrap();
        let relative = current.strip_prefix(root).unwrap().to_path_buf();
        if metadata.is_dir() {
            result.insert(relative, None);
            for entry in fs::read_dir(current).unwrap() {
                visit(root, &entry.unwrap().path(), result);
            }
        } else {
            assert!(metadata.is_file());
            result.insert(relative, Some(fs::read(current).unwrap()));
        }
    }

    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

#[test]
fn production_cli_help_and_unicode_source_flow_use_project_authority() {
    let help = Command::new(source_binary())
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    let help_text = String::from_utf8(help.stdout).unwrap();
    assert!(help_text.contains("orr_pkg_source"));
    assert!(help_text.contains("--source"));
    let usage_error = Command::new(source_binary()).output().unwrap();
    assert_eq!(usage_error.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&usage_error.stderr).contains("orr_pkg_source --source"));

    let directory = tempfile::tempdir().unwrap();
    let first_source = fixture(directory.path(), "模型 source one");
    let first_arguments = arguments(&first_source, &["assets/room.glb", "LICENSE.txt"]);
    let created = run(&first_arguments);
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let created_stdout = String::from_utf8(created.stdout).unwrap();
    assert!(created_stdout.contains("created manifest:"));
    assert!(created_stdout.contains("declared files: 2"));

    let manifest_path = first_source.join(MANIFEST);
    let manifest_bytes = fs::read(&manifest_path).unwrap();
    let inspection =
        inspect_for_install(&first_source, Runtime::content_only().engine_version).unwrap();
    assert_eq!(inspection.manifest.name, "room-art");
    assert_eq!(inspection.manifest.version, "1.0.0");
    assert_eq!(inspection.files.len(), 2);
    assert!(!first_source.join("orr.packages.lock.json").exists());

    let project_root = directory.path().join("game project");
    fs::create_dir(&project_root).unwrap();
    let installer =
        Project::open_for_install(&project_root, Runtime::content_only().engine_version).unwrap();
    installer
        .install(std::slice::from_ref(&first_source))
        .unwrap();
    let consumer = Project::open(&project_root, Runtime::content_only()).unwrap();
    assert_eq!(consumer.verify().unwrap().packages.len(), 1);
    assert_eq!(
        consumer.read_asset("room-art", "assets/room.glb").unwrap(),
        b"room model bytes"
    );
    assert_eq!(
        consumer.read_asset("room-art", "LICENSE.txt").unwrap(),
        b"license bytes"
    );

    let duplicate = run(&first_arguments);
    assert_eq!(duplicate.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("already exists"));
    assert_eq!(fs::read(&manifest_path).unwrap(), manifest_bytes);

    let second_source = fixture(directory.path(), "模型 source two");
    let second = run(&arguments(
        &second_source,
        &["assets/room.glb", "LICENSE.txt"],
    ));
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(
        fs::read(second_source.join(MANIFEST)).unwrap(),
        manifest_bytes
    );
}

#[test]
fn production_cli_rejects_bad_inputs_without_changing_source_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let source = fixture(directory.path(), "valid source");
    let before = tree_snapshot(&source);
    let valid = arguments(&source, &["assets/room.glb", "LICENSE.txt"]);

    let mut relative_source = valid.clone();
    relative_source[1] = "relative source".into();
    let mut unknown_option = valid.clone();
    unknown_option.extend(["--unknown".into(), "value".into()]);
    let mut oversized_value = valid.clone();
    oversized_value[3] = "x".repeat(4097).into();
    let invalid_file = arguments(&source, &["../escape.bin"]);
    let mut missing_value = valid.clone();
    missing_value.push("--file".into());

    for invalid in [
        relative_source,
        unknown_option,
        oversized_value,
        invalid_file,
        missing_value,
    ] {
        let result = run(&invalid);
        assert_eq!(result.status.code(), Some(2));
        assert_eq!(tree_snapshot(&source), before);
    }
    assert!(!source.join(MANIFEST).exists());
}
