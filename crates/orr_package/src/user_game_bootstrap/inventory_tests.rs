use super::*;
use std::{
    fs,
    os::unix::fs::{symlink, PermissionsExt},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "orr-bootstrap-inventory-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn write(&self, path: &str, data: &[u8], executable: bool) -> Entry {
        let p = self.0.join(path);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, data).unwrap();
        fs::set_permissions(
            &p,
            fs::Permissions::from_mode(if executable { 0o701 } else { 0o600 }),
        )
        .unwrap();
        Entry {
            path: path.into(),
            mode: if executable { "100755" } else { "100644" }.into(),
            blob: hex(&git_hash("blob", data)),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn git_hash_known_vectors_and_embedded_tree_ordering() {
    assert_eq!(
        hex(&git_hash("blob", b"hello\n")),
        "ce013625030ba8dba906f756967f9e9ca394464a"
    );
    assert_eq!(
        inventory_tree(&[]).unwrap(),
        "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
    );
    for source in [
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tools/user_game_bootstrap_source.json"
        )),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tools/user_game_action_preview_source.json"
        )),
    ] {
        let inventory: Inventory = serde_json::from_str(source).unwrap();
        assert_eq!(inventory_tree(&inventory.files).unwrap(), inventory.tree);
        let mut reversed = inventory.files;
        reversed.reverse();
        assert_eq!(inventory_tree(&reversed).unwrap(), inventory.tree);
    }
}
#[test]
fn duplicate_prefix_and_ambiguous_paths_fail() {
    let item = |path: &str| Entry {
        path: path.into(),
        mode: "100644".into(),
        blob: "0".repeat(40),
    };
    for paths in [["a", "a"], ["a", "a/b"], ["a/b", "a"]] {
        assert!(inventory_tree(&paths.map(item)).is_err());
    }
    for path in [
        "",
        "/a",
        "a/",
        "a//b",
        "a/../b",
        "a/./b",
        ".git/a",
        "a/target/b",
        "a\\b",
        "a\0b",
    ] {
        assert!(inventory_tree(&[item(path)]).is_err(), "{path:?}");
    }
    assert!(inventory_tree(&[item(&"a/".repeat(129))]).is_err());
    assert!(inventory_tree(&[item(&"x".repeat(MAX_PATH + 1))]).is_err());
    let mut bad = item("a");
    bad.mode = "120000".into();
    assert!(inventory_tree(&[bad]).is_err());
    let mut bad = item("a");
    bad.blob = "A".repeat(40);
    assert!(inventory_tree(&[bad]).is_err());
}
#[test]
fn controlled_tree_retains_only_starter_and_accepts_empty_directories() {
    let fixture = Fixture::new();
    let entries = vec![
        fixture.write("Cargo.toml", b"engine", false),
        fixture.write("examples/static-game-starter/src/lib.rs", b"starter", true),
    ];
    fs::create_dir(fixture.0.join("empty")).unwrap();
    fs::create_dir(fixture.0.join("target")).unwrap();
    fs::write(fixture.0.join("target/untracked"), b"ignored").unwrap();
    symlink("/nonexistent", fixture.0.join(".git")).unwrap();
    let snapshots = validate_files(&fixture.0, &entries).unwrap();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots["src/lib.rs"], b"starter");
    fs::write(fixture.0.join("untracked"), b"bad").unwrap();
    assert!(validate_files(&fixture.0, &entries).is_err());
}
#[test]
fn missing_modified_mode_symlink_and_nonregular_fail_closed() {
    let fixture = Fixture::new();
    let item = fixture.write("input", b"abc", false);
    let entries = vec![item];
    fs::write(fixture.0.join("input"), b"abd").unwrap();
    assert!(validate_files(&fixture.0, &entries).is_err());
    fs::write(fixture.0.join("input"), b"abc").unwrap();
    fs::set_permissions(fixture.0.join("input"), fs::Permissions::from_mode(0o700)).unwrap();
    assert!(validate_files(&fixture.0, &entries).is_err());
    fs::remove_file(fixture.0.join("input")).unwrap();
    assert!(validate_files(&fixture.0, &entries).is_err());
    symlink("/nonexistent", fixture.0.join("input")).unwrap();
    assert!(validate_files(&fixture.0, &entries).is_err());
    fs::remove_file(fixture.0.join("input")).unwrap();
    fs::create_dir(fixture.0.join("input")).unwrap();
    assert!(validate_files(&fixture.0, &entries).is_err());
}
#[test]
fn ancestor_symlink_and_oversized_file_are_refused() {
    let fixture = Fixture::new();
    let entries = vec![fixture.write("input", b"a", false)];
    let file = fs::OpenOptions::new()
        .write(true)
        .open(fixture.0.join("input"))
        .unwrap();
    file.set_len((MAX_FILE + 1) as u64).unwrap();
    assert!(validate_files(&fixture.0, &entries).is_err());
    let outer = Fixture::new();
    symlink(&fixture.0, outer.0.join("alias")).unwrap();
    assert!(validate_files(&outer.0.join("alias"), &entries).is_err());
}
#[test]
fn retained_starter_count_is_bounded() {
    let fixture = Fixture::new();
    let entries: Vec<_> = (0..129)
        .map(|i| fixture.write(&format!("{STARTER}{i}"), b"", false))
        .collect();
    assert!(validate_files(&fixture.0, &entries).is_err());
}
