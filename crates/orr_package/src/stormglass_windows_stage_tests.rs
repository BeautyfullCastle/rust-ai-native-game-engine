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
#[test]
fn reservations_refuse_leaf_write_delete_and_directory_rename() {
    let (root, path) = directory();
    let inputs = ReservedInputs::open(&path).unwrap();
    let stage = UnvalidatedStage::prepare(&inputs, &hash(&fixture()), root.path()).unwrap();
    for dir in [&path, &stage.path] {
        assert!(fs::write(dir.join(SCENE_NAME), b"tamper").is_err());
        assert!(fs::remove_file(dir.join(RUNTIME_NAME)).is_err());
        assert!(fs::rename(dir, root.path().join("moved")).is_err());
    }
    let retained = stage.path().to_owned();
    drop(stage);
    fs::write(retained.join(SCENE_NAME), b"released").unwrap();
    assert!(fs::write(path.join(SCENE_NAME), b"still reserved").is_err());
    drop(inputs);
    fs::write(path.join(SCENE_NAME), b"released source").unwrap();
}
#[test]
fn prepared_stage_is_exact_and_drop_retains_all_files() {
    let (root, path) = directory();
    let inputs = ReservedInputs::open(&path).unwrap();
    let mut stage = UnvalidatedStage::prepare(&inputs, &hash(&fixture()), root.path()).unwrap();
    stage.recheck().unwrap();
    let retained = stage.path().to_owned();
    assert_eq!(fs::read(retained.join(RUNTIME_NAME)).unwrap(), fixture());
    assert_eq!(fs::read(retained.join(SCENE_NAME)).unwrap(), SCENE);
    assert_eq!(fs::read(retained.join(LAUNCHER_NAME)).unwrap(), LAUNCHER);
    assert_eq!(fs::read_dir(&retained).unwrap().count(), 3);
    drop(stage);
    assert!(retained.exists());
    fs::write(retained.join(SCENE_NAME), b"released").unwrap();
}
#[test]
fn added_child_is_refused_and_never_cleaned_up() {
    let (root, path) = directory();
    let inputs = ReservedInputs::open(&path).unwrap();
    let mut stage = UnvalidatedStage::prepare(&inputs, &hash(&fixture()), root.path()).unwrap();
    let retained = stage.path().to_owned();
    fs::write(retained.join("foreign"), b"keep").unwrap();
    assert!(stage.recheck().unwrap_err().contains("closure"));
    drop(stage);
    assert_eq!(fs::read(retained.join("foreign")).unwrap(), b"keep");
}
#[test]
fn bad_pin_leaves_parent_and_preexisting_files_unchanged() {
    let (root, path) = directory();
    fs::write(root.path().join("sentinel"), b"keep").unwrap();
    let inputs = ReservedInputs::open(&path).unwrap();
    let count = fs::read_dir(root.path()).unwrap().count();
    assert!(UnvalidatedStage::prepare(&inputs, &"0".repeat(64), root.path()).is_err());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), count);
    assert_eq!(fs::read(root.path().join("sentinel")).unwrap(), b"keep");
}
#[test]
fn simultaneous_preparations_have_separate_stages() {
    let (root, path) = directory();
    let inputs = ReservedInputs::open(&path).unwrap();
    let a = UnvalidatedStage::prepare(&inputs, &hash(&fixture()), root.path()).unwrap();
    let b = UnvalidatedStage::prepare(&inputs, &hash(&fixture()), root.path()).unwrap();
    assert_ne!(a.path(), b.path());
    assert_eq!(
        fs::read(a.path().join(RUNTIME_NAME)).unwrap(),
        fs::read(b.path().join(RUNTIME_NAME)).unwrap()
    );
}
#[test]
fn partial_failure_reports_stage_and_preserves_collision() {
    let (root, path) = directory();
    let inputs = ReservedInputs::open(&path).unwrap();
    let mut retained = PathBuf::new();
    let result = UnvalidatedStage::prepare_with(&inputs, &hash(&fixture()), root.path(), |stage| {
        retained = stage.to_owned();
        fs::write(stage.join(SCENE_NAME), b"preexisting").unwrap();
        Ok(())
    });
    let error = result.err().unwrap();
    assert!(error.contains(retained.to_str().unwrap()));
    assert_eq!(fs::read(retained.join(SCENE_NAME)).unwrap(), b"preexisting");
    assert_eq!(fs::read(retained.join(RUNTIME_NAME)).unwrap(), fixture());
    // Error releases held stage handles without cleanup or permission changes.
    fs::write(retained.join(RUNTIME_NAME), b"released").unwrap();
    assert_eq!(fs::read(path.join(SCENE_NAME)).unwrap(), SCENE);
}
