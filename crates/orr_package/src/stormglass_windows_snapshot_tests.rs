use super::*;
use sha2::{Digest, Sha256};
use std::fs;
const SCENE: &[u8] = b"schema: orr.scene/1\nentities: {}\n";
fn fixture() -> Vec<u8> {
    let mut b = vec![0; 1024];
    b[..2].copy_from_slice(b"MZ");
    b[64..68].copy_from_slice(b"PE\0\0");
    for (offset, value) in [
        (68, 0x8664u16),
        (70, 1),
        (84, 240),
        (86, 2),
        (88, 0x20b),
        (156, 3),
    ] {
        b[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }
    for (offset, value) in [
        (0x3c, 64u32),
        (104, 4096),
        (148, 512),
        (340, 4096),
        (344, 512),
        (348, 512),
        (364, 0x20000000),
    ] {
        b[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    b
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn directory() -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("inputs");
    fs::create_dir(&path).unwrap();
    fs::write(path.join(RUNTIME_NAME), fixture()).unwrap();
    fs::write(path.join(SCENE_NAME), SCENE).unwrap();
    (root, path)
}
fn junction(path: &Path, target: &Path) {
    let result = std::process::Command::new("cmd.exe")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(path)
        .arg(target)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
#[test]
fn captured_inputs_match_content_admission_without_writes() {
    let (_root, path) = directory();
    fs::write(path.join("unrelated.txt"), b"preserve").unwrap();
    let permissions = fs::metadata(path.join(RUNTIME_NAME)).unwrap().permissions();
    let reserved = ReservedInputs::open(&path).unwrap();
    let admitted = reserved.admit(&hash(&fixture())).unwrap();
    let bytes = fixture();
    let pure = admit(&bytes, &hash(&bytes), SCENE).unwrap();
    assert_eq!(admitted.files(), pure.files());
    assert_eq!(admitted.config(), pure.config());
    assert_eq!(admitted.runtime(), bytes);
    assert_eq!(admitted.scene(), SCENE);
    assert_eq!(fs::read_dir(&path).unwrap().count(), 3);
    assert_eq!(fs::read(path.join("unrelated.txt")).unwrap(), b"preserve");
    assert_eq!(
        fs::metadata(path.join(RUNTIME_NAME)).unwrap().permissions(),
        permissions
    );
}
#[test]
fn rejects_ambiguous_and_device_paths() {
    for path in [
        r"relative",
        r"C:relative",
        r"C:\",
        r"\\server\share\inputs",
        r"\\?\C:\inputs",
        r"\\.\C:\inputs",
        r"C:\a\..\b",
        r"C:\a\.\b",
        r"C:\a.",
        r"C:\a ",
        r"C:\NUL.txt",
        r"C:\CON .txt",
        r"C:\COM1",
        r"C:\LPT¹",
        r"C:\COM²",
        r"C:\LPT³",
        r"C:\a:stream",
        r"C:/inputs",
        r"C:\a\",
        r"C:\a\\b",
        "C:\\a\0",
    ] {
        assert!(input_parts(Path::new(path)).is_err(), "{path:?}");
    }
    let invalid = std::ffi::OsString::from_wide(&[67, 58, 92, 0xd800]);
    assert!(input_parts(Path::new(&invalid)).is_err());
    assert!(input_parts(Path::new(&format!(r"C:\{}", "a".repeat(240)))).is_err());
}
#[test]
fn reservations_block_writes_delete_renames_and_ancestor_replacement_until_drop() {
    let (root, path) = directory();
    let reserved = ReservedInputs::open(&path).unwrap();
    reserved.admit(&hash(&fixture())).unwrap();
    for name in [RUNTIME_NAME, SCENE_NAME] {
        let input = path.join(name);
        assert!(OpenOptions::new().write(true).open(&input).is_err());
        assert!(fs::remove_file(&input).is_err());
        assert!(fs::rename(&input, path.join("moved")).is_err());
        assert_eq!(
            fs::read(&input).unwrap(),
            if name == RUNTIME_NAME {
                fixture()
            } else {
                SCENE.to_vec()
            }
        );
    }
    assert!(fs::rename(&path, root.path().join("moved")).is_err());
    assert!(fs::rename(root.path(), root.path().with_extension("moved")).is_err());
    drop(reserved);
    OpenOptions::new()
        .write(true)
        .open(path.join(RUNTIME_NAME))
        .unwrap();
    fs::rename(&path, root.path().join("moved")).unwrap();
}
#[test]
fn existing_write_and_delete_handles_fail_closed_even_with_generous_sharing() {
    let (_root, path) = directory();
    for name in [RUNTIME_NAME, SCENE_NAME] {
        for access in [0x40000000, 0x00010000] {
            // GENERIC_WRITE, DELETE
            let held = OpenOptions::new()
                .access_mode(access)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                .open(path.join(name))
                .unwrap();
            assert!(ReservedInputs::open(&path).is_err());
            drop(held);
            assert!(ReservedInputs::open(&path).is_ok());
        }
    }
}
#[test]
fn rejects_junction_ancestry_and_leaf_preserving_targets() {
    let (root, path) = directory();
    let alias = root.path().join("alias");
    junction(&alias, &path);
    assert!(ReservedInputs::open(&alias).is_err());
    fs::create_dir(path.join("child")).unwrap();
    assert!(ReservedInputs::open(&alias.join("child")).is_err());
    fs::remove_dir(&alias).unwrap();
    for name in [RUNTIME_NAME, SCENE_NAME] {
        let input = path.join(name);
        let saved = root.path().join("saved");
        fs::rename(&input, &saved).unwrap();
        let before = fs::read(&saved).unwrap();
        junction(&input, root.path());
        assert!(ReservedInputs::open(&path).is_err());
        assert_eq!(fs::read(&saved).unwrap(), before);
        fs::remove_dir(&input).unwrap();
        fs::rename(&saved, &input).unwrap();
    }
}
#[test]
fn rejects_hardlinked_missing_nonregular_and_oversize_inputs_without_modification() {
    let (root, path) = directory();
    for (name, max) in [
        (RUNTIME_NAME, MAX_RUNTIME_BYTES),
        (SCENE_NAME, MAX_SOURCE_BYTES),
    ] {
        let input = path.join(name);
        let saved = root.path().join("saved");
        fs::rename(&input, &saved).unwrap();
        let before = fs::read(&saved).unwrap();
        fs::hard_link(&saved, &input).unwrap();
        assert!(ReservedInputs::open(&path).is_err());
        assert_eq!(fs::read(&saved).unwrap(), before);
        fs::remove_file(&input).unwrap();
        assert!(ReservedInputs::open(&path).is_err());
        fs::create_dir(&input).unwrap();
        assert!(ReservedInputs::open(&path).is_err());
        fs::remove_dir(&input).unwrap();
        let file = File::create(&input).unwrap();
        file.set_len(max as u64 + 1).unwrap();
        drop(file);
        assert!(ReservedInputs::open(&path).is_err());
        assert_eq!(fs::metadata(&input).unwrap().len(), max as u64 + 1);
        fs::remove_file(&input).unwrap();
        fs::rename(saved, input).unwrap();
    }
}
#[test]
fn capture_and_admission_failures_release_all_owned_reservations() {
    let (root, path) = directory();
    assert!(ReservedInputs::open_with(&path, |_| Err("injected failure".into())).is_err());
    OpenOptions::new()
        .write(true)
        .open(path.join(RUNTIME_NAME))
        .unwrap();
    let reserved = ReservedInputs::open(&path).unwrap();
    assert!(reserved.admit(&"0".repeat(64)).is_err());
    assert!(OpenOptions::new()
        .write(true)
        .open(path.join(SCENE_NAME))
        .is_err());
    drop(reserved);
    OpenOptions::new()
        .write(true)
        .open(path.join(SCENE_NAME))
        .unwrap();
    fs::rename(&path, root.path().join("moved")).unwrap();
}
#[test]
fn mutation_before_capture_is_seen_and_mutation_after_capture_is_blocked() {
    let (_root, path) = directory();
    let reserved = ReservedInputs::open_with(&path, |name| {
        if name == RUNTIME_NAME {
            assert!(fs::write(path.join(RUNTIME_NAME), b"changed").is_err());
            fs::write(path.join(SCENE_NAME), b"invalid: true").unwrap();
        } else {
            assert!(fs::write(path.join(SCENE_NAME), SCENE).is_err());
        }
        Ok(())
    })
    .unwrap();
    assert!(reserved.admit(&hash(&fixture())).is_err());
    assert_eq!(fs::read(path.join(RUNTIME_NAME)).unwrap(), fixture());
    assert_eq!(fs::read(path.join(SCENE_NAME)).unwrap(), b"invalid: true");
}
#[test]
fn existing_ancestor_write_and_delete_handles_conflict() {
    let (root, path) = directory();
    for directory in [root.path(), path.as_path()] {
        for access in [0x40000000, 0x00010000] {
            let held = OpenOptions::new()
                .access_mode(access)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                .open(directory)
                .unwrap();
            assert!(ReservedInputs::open(&path).is_err());
            drop(held);
            assert!(ReservedInputs::open(&path).is_ok());
        }
    }
}
