use orr_asset::AssetRef;
use orr_asset_cook::authoring::{Entry, Index};
use std::fs;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
static SERIAL: AtomicU64 = AtomicU64::new(0);
struct Directory(std::path::PathBuf);
impl Directory {
    fn new() -> Self {
        for _ in 0..128 {
            let path = std::env::temp_dir().join(format!(
                "orr-asset-cli-test-{}-{}",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
                Err(e) => panic!("test directory: {e}"),
            }
        }
        panic!("no test directory")
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_orr_asset_cook"))
            .args(args)
            .current_dir(&self.0)
            .output()
            .unwrap()
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn cli_register_clone_move_and_tombstone_preserve_old_files() {
    let dir = Directory::new();
    fs::write(dir.0.join("empty.json"), Index::default().encode().unwrap()).unwrap();
    let register = dir.run(&[
        "register",
        "--index",
        "empty.json",
        "--out-index",
        "live.json",
        "--type",
        "sim.motion_profile",
        "--source",
        "source/motion.json",
    ]);
    assert!(
        register.status.success(),
        "{}",
        String::from_utf8_lossy(&register.stderr)
    );
    let first: AssetRef = String::from_utf8(register.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(!first.is_null());
    let clone = dir.run(&[
        "clone",
        "--index",
        "live.json",
        "--out-index",
        "clone.json",
        "--from",
        &first.to_string(),
        "--source",
        "source/copy.json",
        "--id",
        "a_ffffffffffffffff",
    ]);
    assert!(clone.status.success());
    let moved = dir.run(&[
        "move",
        "--index",
        "clone.json",
        "--out-index",
        "moved.json",
        "--id",
        "a_ffffffffffffffff",
        "--source",
        "renamed.json",
    ]);
    assert!(moved.status.success());
    let removed = dir.run(&[
        "tombstone",
        "--index",
        "moved.json",
        "--out-index",
        "removed.json",
        "--id",
        "a_ffffffffffffffff",
        "--root",
        &first.to_string(),
    ]);
    assert!(removed.status.success());
    let tombstones = Index::parse(&fs::read(dir.0.join("removed.json")).unwrap()).unwrap();
    assert!(tombstones
        .entries
        .iter()
        .any(|entry| matches!(entry, Entry::Tombstone { id } if id.get() == u64::MAX)));
    let before = fs::read(dir.0.join("live.json")).unwrap();
    let overwrite = dir.run(&[
        "register",
        "--index",
        "empty.json",
        "--out-index",
        "live.json",
        "--type",
        "sim.motion_profile",
        "--source",
        "other.json",
    ]);
    assert!(!overwrite.status.success());
    assert_eq!(fs::read(dir.0.join("live.json")).unwrap(), before);
    assert_eq!(
        Index::parse(&fs::read(dir.0.join("empty.json")).unwrap())
            .unwrap()
            .entries
            .len(),
        0
    );
}

#[test]
fn cli_usage_errors_do_not_create_outputs() {
    let dir = Directory::new();
    fs::write(dir.0.join("empty.json"), Index::default().encode().unwrap()).unwrap();
    for args in [
        vec![
            "cook",
            "--index",
            "empty.json",
            "--out",
            "out",
            "--unknown",
            "value",
        ],
        vec![
            "cook",
            "--index",
            "empty.json",
            "--index",
            "empty.json",
            "--out",
            "out",
        ],
        vec!["cook", "--index", "empty.json", "--out"],
        vec![
            "move",
            "--index",
            "empty.json",
            "--out-index",
            "out",
            "--source",
            "source.json",
        ],
    ] {
        assert!(!dir.run(&args).status.success());
        assert!(!dir.0.join("out").exists());
    }
    assert!(dir.run(&["--help"]).status.success());
}
