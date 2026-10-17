#![cfg(all(windows, feature = "project-create", feature = "room-project"))]

use orr_sample::room_project::PreparedProject;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn creator() -> Command {
    Command::new(env!("CARGO_BIN_EXE_orr_new_room"))
}

fn run(arguments: &[&str]) -> Output {
    creator().args(arguments).output().unwrap()
}

fn ordinary_windows_path(path: &Path) -> PathBuf {
    let text = path.to_str().expect("temporary path must be Unicode");
    PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(text))
}

fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn visit(root: &Path, directory: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let file_type = entry.file_type().unwrap();
            assert!(!file_type.is_symlink());
            if file_type.is_dir() {
                visit(root, &entry.path(), files);
            } else {
                assert!(file_type.is_file());
                let relative = entry
                    .path()
                    .strip_prefix(root)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned();
                files.insert(relative, fs::read(entry.path()).unwrap());
            }
        }
    }

    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

#[test]
fn production_cli_help_and_invalid_arguments_have_distinct_exit_codes() {
    let help = run(&["--help"]);
    assert_eq!(help.status.code(), Some(0));
    let help_text = String::from_utf8(help.stdout).unwrap();
    assert!(help_text.contains("orr_new_room"), "{help_text}");
    assert!(help_text.contains("--output"), "{help_text}");
    assert!(help_text.contains("--seed"), "{help_text}");

    for arguments in [
        vec![],
        vec!["--output", "C:\\Room"],
        vec!["--output", "C:\\Room", "--seed", "bad seed"],
        vec!["--output", "C:\\Room", "--seed", "seed", "--seed", "other"],
        vec!["--output", "C:\\Room", "--mystery", "seed"],
    ] {
        let result = run(&arguments);
        assert_eq!(result.status.code(), Some(2), "{arguments:?}: {result:?}");
    }
}

#[test]
fn production_cli_creates_deterministic_admitted_unicode_projects() {
    let temp = tempfile::tempdir().unwrap();
    let root = ordinary_windows_path(temp.path());
    let first = root.join("Room project Ω");
    let second = root.join("Second Room project Ω");
    for output in [&first, &second] {
        let result = creator()
            .args(["--output"])
            .arg(output)
            .args(["--seed", "same-seed"])
            .output()
            .unwrap();
        assert_eq!(
            result.status.code(),
            Some(0),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(String::from_utf8_lossy(&result.stdout).contains("created project"));
    }

    assert_eq!(tree(&first), tree(&second));
    let prepared = PreparedProject::open(&first).unwrap();
    assert_eq!(prepared.scene().frame().entities().count(), 7);
    assert_eq!(prepared.models().document.bindings.len(), 2);
    assert!(prepared.camera().is_some());
    assert_eq!(
        orr_package::Project::open(&first, orr_sample::room_project::compiled_runtime())
            .unwrap()
            .verify()
            .unwrap()
            .packages
            .len(),
        1
    );
}

#[test]
fn production_cli_creation_failures_preserve_destinations_and_source_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let root = ordinary_windows_path(temp.path());
    let existing_dir = root.join("existing directory");
    fs::create_dir(&existing_dir).unwrap();
    fs::write(existing_dir.join("keep.txt"), b"directory owner").unwrap();
    let existing_file = root.join("existing file");
    fs::write(&existing_file, b"file owner").unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/imported_scene_demo");
    let source = source.canonicalize().unwrap();
    let source_before = tree(&source);

    for output in [
        existing_dir.clone(),
        existing_file.clone(),
        root.join("missing parent").join("Room"),
        PathBuf::from("relative-room-project"),
        source.clone(),
    ] {
        let result = creator()
            .args(["--output"])
            .arg(&output)
            .args(["--seed", "valid-seed"])
            .output()
            .unwrap();
        assert_eq!(
            result.status.code(),
            Some(1),
            "{}: {}",
            output.display(),
            String::from_utf8_lossy(&result.stderr)
        );
    }

    assert_eq!(
        fs::read(existing_dir.join("keep.txt")).unwrap(),
        b"directory owner"
    );
    assert_eq!(fs::read(existing_file).unwrap(), b"file owner");
    assert!(!root.join("missing parent").exists());
    assert_eq!(tree(&source), source_before);
}

#[test]
fn production_cli_rejects_a_reparse_parent() {
    let temp = tempfile::tempdir().unwrap();
    let root = ordinary_windows_path(temp.path());
    let target = root.join("real parent");
    fs::create_dir(&target).unwrap();
    let reparse = root.join("reparse parent");
    let junction = Command::new("cmd.exe")
        .args(["/C", "mklink", "/J"])
        .arg(&reparse)
        .arg(&target)
        .output()
        .unwrap();
    assert!(
        junction.status.success(),
        "could not create Windows junction: {}",
        String::from_utf8_lossy(&junction.stderr)
    );

    let output = reparse.join("Room");
    let result = creator()
        .args(["--output"])
        .arg(&output)
        .args(["--seed", "valid-seed"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(!target.join("Room").exists());
    assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
}
