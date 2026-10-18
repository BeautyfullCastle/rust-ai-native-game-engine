use super::*;
use crate::mesh_lighting::Document;

fn bytes() -> Vec<u8> {
    Preset::from_document(&Document::from_lighting(orr_render::Lighting::default()).unwrap())
        .unwrap()
        .encode()
        .unwrap()
}

fn root() -> (tempfile::TempDir, PathBuf) {
    let temporary = tempfile::tempdir().unwrap();
    // Unix temp roots may be aliases (for example macOS /var). Windows
    // canonicalize adds a verbatim device prefix, intentionally not admitted.
    #[cfg(not(windows))]
    let path = temporary.path().canonicalize().unwrap();
    #[cfg(windows)]
    let path = temporary.path().to_owned();
    (temporary, path)
}

#[test]
fn save_new_and_load_roundtrip_with_no_staging_files_left_behind() {
    let (_temporary, root) = root();
    let path = root.join("warm.mesh-lighting-preset.json");
    save_new(&path, &bytes()).unwrap();
    assert_eq!(load(&path).unwrap(), Preset::decode(&bytes()).unwrap());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
}

#[test]
fn existing_file_and_directory_are_never_overwritten() {
    let (_temporary, root) = root();
    let path = root.join("existing.mesh-lighting-preset.json");
    fs::write(&path, b"existing user data").unwrap();
    assert!(save_new(&path, &bytes())
        .unwrap_err()
        .contains("already exists"));
    assert_eq!(fs::read(&path).unwrap(), b"existing user data");
    let directory = root.join("directory.mesh-lighting-preset.json");
    fs::create_dir(&directory).unwrap();
    assert!(save_new(&directory, &bytes()).is_err());
    assert!(directory.is_dir());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
}

#[test]
fn malformed_or_oversized_save_writes_nothing() {
    let (_temporary, root) = root();
    let path = root.join("invalid.mesh-lighting-preset.json");
    for invalid in [b"{}".to_vec(), Vec::new(), vec![b' '; MAX_BYTES + 1]] {
        assert!(save_new(&path, &invalid).is_err());
        assert!(!path.exists());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    }
}

#[test]
fn load_rejects_missing_malformed_oversized_and_nonregular_files() {
    let (_temporary, root) = root();
    let path = root.join("input.mesh-lighting-preset.json");
    assert!(load(&path).is_err());
    fs::write(&path, b"{}").unwrap();
    assert!(load(&path).is_err());
    fs::write(&path, vec![b' '; MAX_BYTES + 1]).unwrap();
    assert!(load(&path).is_err());
    let directory = root.join("folder.mesh-lighting-preset.json");
    fs::create_dir(&directory).unwrap();
    assert!(load(&directory).is_err());
}

#[test]
fn path_policy_rejects_relative_traversal_devices_and_wrong_extensions() {
    let (_temporary, root) = root();
    for path in [
        PathBuf::from("relative.mesh-lighting-preset.json"),
        root.join("wrong.json"),
        root.join(".mesh-lighting-preset.json"),
        root.join("CON.mesh-lighting-preset.json"),
        root.join("x:stream.mesh-lighting-preset.json"),
        root.join("..").join("escape.mesh-lighting-preset.json"),
        root.join("missing").join("x.mesh-lighting-preset.json"),
    ] {
        assert!(save_new(&path, &bytes()).is_err(), "{}", path.display());
        assert!(load(&path).is_err(), "{}", path.display());
    }
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
}

#[test]
fn changes_after_read_are_rejected() {
    let (_temporary, root) = root();
    let path = root.join("changes.mesh-lighting-preset.json");
    fs::write(&path, bytes()).unwrap();
    let checked = checked_path(&path).unwrap();
    let error = read_file(
        &checked,
        || {},
        || {
            OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap()
                .write_all(b" ")
                .unwrap();
        },
    )
    .unwrap_err();
    assert!(error.contains("changed"));
}

#[test]
fn replacement_between_inspection_and_open_is_rejected() {
    let (_temporary, root) = root();
    let path = root.join("replace.mesh-lighting-preset.json");
    fs::write(&path, bytes()).unwrap();
    let checked = checked_path(&path).unwrap();
    let staged = root.join("replacement");
    // Different length makes the observation deterministic even on Windows
    // filesystems whose creation/write timestamp granularity is coarse.
    let mut replacement = bytes();
    replacement.push(b' ');
    fs::write(&staged, replacement).unwrap();
    let error = read_file(
        &checked,
        || {
            fs::remove_file(&path).unwrap();
            fs::rename(&staged, &path).unwrap();
        },
        || {},
    )
    .unwrap_err();
    assert!(error.contains("changed"));
}

#[cfg(unix)]
#[test]
fn symlink_leaf_parent_and_dangling_destination_are_rejected() {
    use std::os::unix::fs::symlink;
    let (_temporary, root) = root();
    let target = root.join("target.mesh-lighting-preset.json");
    fs::write(&target, bytes()).unwrap();
    let link = root.join("link.mesh-lighting-preset.json");
    symlink(&target, &link).unwrap();
    assert!(load(&link).is_err());
    assert!(save_new(&link, &bytes()).is_err());
    assert_eq!(fs::read(&target).unwrap(), bytes());
    let dangling = root.join("dangling.mesh-lighting-preset.json");
    symlink(root.join("absent"), &dangling).unwrap();
    assert!(save_new(&dangling, &bytes()).is_err());
    assert!(fs::symlink_metadata(&dangling)
        .unwrap()
        .file_type()
        .is_symlink());
    let actual_dir = root.join("real");
    fs::create_dir(&actual_dir).unwrap();
    let linked_dir = root.join("linked");
    symlink(&actual_dir, &linked_dir).unwrap();
    assert!(save_new(&linked_dir.join("x.mesh-lighting-preset.json"), &bytes()).is_err());
    assert_eq!(fs::read_dir(&actual_dir).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn symlink_replacement_after_inspection_is_rejected() {
    use std::os::unix::fs::symlink;
    let (_temporary, root) = root();
    let path = root.join("swap.mesh-lighting-preset.json");
    let other = root.join("other.mesh-lighting-preset.json");
    fs::write(&path, bytes()).unwrap();
    fs::write(&other, bytes()).unwrap();
    let checked = checked_path(&path).unwrap();
    assert!(read_file(
        &checked,
        || {
            fs::remove_file(&path).unwrap();
            symlink(&other, &path).unwrap();
        },
        || {}
    )
    .is_err());
}

#[cfg(unix)]
#[test]
fn changed_metadata_even_with_unchanged_length_is_rejected() {
    use std::os::unix::fs::PermissionsExt;
    let (_temporary, root) = root();
    let path = root.join("mode.mesh-lighting-preset.json");
    fs::write(&path, bytes()).unwrap();
    let mode = fs::metadata(&path).unwrap().permissions().mode();
    let checked = checked_path(&path).unwrap();
    assert!(read_file(
        &checked,
        || {},
        || {
            fs::set_permissions(&path, fs::Permissions::from_mode(mode ^ 0o100)).unwrap();
        }
    )
    .is_err());
}

#[cfg(unix)]
#[test]
fn replaced_parent_is_detected_before_publication() {
    let (_temporary, root) = root();
    let parent = root.join("parent");
    fs::create_dir(&parent).unwrap();
    let checked = checked_path(&parent.join("x.mesh-lighting-preset.json")).unwrap();
    fs::rename(&parent, root.join("moved")).unwrap();
    fs::create_dir(&parent).unwrap();
    assert!(recheck_ancestors(&checked).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn fifo_replacement_is_rejected_without_a_blocking_open() {
    const CHILD: &str = "ORR_MESH_LIGHTING_PRESET_FIFO_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let (_temporary, root) = root();
        let path = root.join("fifo.mesh-lighting-preset.json");
        fs::write(&path, bytes()).unwrap();
        let checked = checked_path(&path).unwrap();
        assert!(read_file(
            &checked,
            || {
                fs::remove_file(&path).unwrap();
                assert!(std::process::Command::new("mkfifo")
                    .arg(&path)
                    .status()
                    .unwrap()
                    .success());
            },
            || {}
        )
        .is_err());
        assert!(load(&path).is_err());
        assert!(save_new(&path, &bytes()).is_err());
        return;
    }
    let name =
        "mesh_lighting_preset::store::tests::fifo_replacement_is_rejected_without_a_blocking_open";
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture", "--test-threads=1"])
        .env(CHILD, "1")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            let output = child.wait_with_output().unwrap();
            assert!(
                status.success(),
                "FIFO child failed: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            break;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!(
                "preset load hung opening a swapped FIFO: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn a_destination_created_during_staging_is_not_replaced_or_removed() {
    let (_temporary, root) = root();
    let path = root.join("race.mesh-lighting-preset.json");
    let error = save_new_observed(&path, &bytes(), || {
        fs::write(&path, b"another writer's file").unwrap();
    })
    .unwrap_err();
    assert!(error.contains("without replacement"));
    assert_eq!(fs::read(&path).unwrap(), b"another writer's file");
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
}

#[test]
fn replacement_after_read_is_rejected() {
    let (_temporary, root) = root();
    let path = root.join("replace-after.mesh-lighting-preset.json");
    fs::write(&path, bytes()).unwrap();
    let staged = root.join("replacement");
    // Different length makes the observation deterministic even on Windows
    // filesystems whose creation/write timestamp granularity is coarse.
    let mut replacement = bytes();
    replacement.push(b' ');
    fs::write(&staged, replacement).unwrap();
    let checked = checked_path(&path).unwrap();
    assert!(read_file(
        &checked,
        || {},
        || {
            fs::remove_file(&path).unwrap();
            fs::rename(&staged, &path).unwrap();
        }
    )
    .is_err());
}
