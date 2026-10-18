use orr_package::{Lock, Project, Runtime};
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

const LOCK: &str = "orr.packages.lock.json";

fn source(root: &Path, name: &str, dependencies: &[(&str, &str)]) -> PathBuf {
    let path = root.join(name);
    fs::create_dir(&path).unwrap();
    let dependencies: BTreeMap<_, _> = dependencies.iter().copied().collect();
    fs::write(
        path.join("orr.package.json"),
        serde_json::to_vec(&json!({
            "schema":1, "name":name, "version":"1.0.0", "engine":"^0.0.1",
            "capabilities":[], "dependencies":dependencies, "files":["asset.txt"]
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(path.join("asset.txt"), name).unwrap();
    path
}

fn project(root: &Path) -> Project {
    let path = root.join("project");
    fs::create_dir(&path).unwrap();
    Project::open(&path, Runtime::content_only()).unwrap()
}

fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().into(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

#[test]
fn checked_remove_preserves_sources_cached_objects_and_supports_restore() {
    let dir = tempfile::tempdir().unwrap();
    let art = source(dir.path(), "art", &[]);
    let p = project(dir.path());
    let old = p.install(&[art]).unwrap();
    let before = files(dir.path());
    let preview = p.preview_remove("art").unwrap();
    let removed = p
        .remove_checked("art", &preview.previous, &preview.candidate, || Ok(()))
        .unwrap();
    assert_eq!(removed, Lock::default());
    let mut after = files(dir.path());
    after.insert(
        PathBuf::from("project").join(LOCK),
        before[&PathBuf::from("project").join(LOCK)].clone(),
    );
    assert_eq!(after, before);
    assert_eq!(
        p.restore_cached("art", &old.packages["art"].digest)
            .unwrap(),
        old
    );
}

#[test]
fn checked_remove_preserves_shared_dependency_and_other_direct_root() {
    let dir = tempfile::tempdir().unwrap();
    let shared = source(dir.path(), "shared", &[]);
    let a = source(dir.path(), "a", &[("shared", "1.0.0")]);
    let b = source(dir.path(), "b", &[("shared", "1.0.0")]);
    let p = project(dir.path());
    p.install_with_dependencies(&[a, b], &[shared]).unwrap();
    let preview = p.preview_remove("a").unwrap();
    let removed = p
        .remove_checked("a", &preview.previous, &preview.candidate, || Ok(()))
        .unwrap();
    assert!(!removed.packages.contains_key("a"));
    assert!(removed.direct.contains_key("b"));
    assert!(!removed.direct.contains_key("shared"));
    assert_eq!(
        removed.packages["shared"],
        preview.previous.packages["shared"]
    );
}

#[test]
fn checked_remove_direct_selection_can_remain_transitively_active() {
    let dir = tempfile::tempdir().unwrap();
    let shared = source(dir.path(), "shared", &[]);
    let a = source(dir.path(), "a", &[("shared", "1.0.0")]);
    let p = project(dir.path());
    p.install(&[a, shared]).unwrap();
    let preview = p.preview_remove("shared").unwrap();
    let removed = p
        .remove_checked("shared", &preview.previous, &preview.candidate, || Ok(()))
        .unwrap();
    assert!(!removed.direct.contains_key("shared"));
    assert_eq!(removed.packages, preview.previous.packages);
    assert!(p
        .remove_checked("shared", &removed, &removed, || panic!(
            "transitive callback"
        ))
        .is_err());
    assert_eq!(p.verify().unwrap(), removed);
}

#[test]
fn checked_remove_rejects_stale_and_unapproved_locks_before_callback() {
    let dir = tempfile::tempdir().unwrap();
    let a = source(dir.path(), "a", &[]);
    let b = source(dir.path(), "b", &[]);
    let p = project(dir.path());
    p.install(&[a]).unwrap();
    let preview = p.preview_remove("a").unwrap();
    assert!(p
        .remove_checked("a", &preview.previous, &preview.previous, || panic!(
            "unapproved callback"
        ))
        .is_err());
    p.install(&[b]).unwrap();
    let before = files(dir.path());
    assert!(p
        .remove_checked("a", &preview.previous, &preview.candidate, || panic!(
            "stale callback"
        ))
        .is_err());
    assert_eq!(files(dir.path()), before);
}

#[test]
fn checked_remove_callback_failure_is_atomic_and_writer_guard_is_held() {
    let dir = tempfile::tempdir().unwrap();
    let a = source(dir.path(), "a", &[]);
    let p = project(dir.path());
    p.install(&[a]).unwrap();
    let preview = p.preview_remove("a").unwrap();
    let before = files(dir.path());
    let failure = p
        .remove_checked("a", &preview.previous, &preview.candidate, || {
            assert_eq!(p.verify().unwrap(), preview.previous);
            assert!(p.remove("a").is_err());
            assert!(p
                .remove_checked("a", &preview.previous, &preview.candidate, || panic!(
                    "competing callback"
                ))
                .is_err());
            Err("saved model references changed".into())
        })
        .unwrap_err();
    assert!(failure
        .to_string()
        .contains("saved model references changed"));
    assert_eq!(files(dir.path()), before);
    assert_eq!(p.verify().unwrap(), preview.previous);
    assert_eq!(
        p.remove_checked("a", &preview.previous, &preview.candidate, || Ok(()))
            .unwrap(),
        preview.candidate
    );
}

#[test]
fn checked_remove_preserves_existing_writer_and_active_authority() {
    let dir = tempfile::tempdir().unwrap();
    let a = source(dir.path(), "a", &[]);
    let p = project(dir.path());
    p.install(&[a]).unwrap();
    let preview = p.preview_remove("a").unwrap();
    fs::write(
        p.root().join(".orr/packages/writer.lock"),
        "existing writer",
    )
    .unwrap();
    let before = files(dir.path());
    assert!(p
        .remove_checked("a", &preview.previous, &preview.candidate, || panic!(
            "locked callback"
        ))
        .is_err());
    assert_eq!(files(dir.path()), before);
}

#[test]
fn checked_remove_rejects_corrupt_content_even_when_deactivating_it() {
    let dir = tempfile::tempdir().unwrap();
    let a = source(dir.path(), "a", &[]);
    let p = project(dir.path());
    p.install(&[a]).unwrap();
    let preview = p.preview_remove("a").unwrap();
    fs::write(
        p.root()
            .join(".orr/packages/objects")
            .join(&preview.previous.packages["a"].digest)
            .join("asset.txt"),
        "tampered",
    )
    .unwrap();
    let before = files(dir.path());
    assert!(p
        .remove_checked("a", &preview.previous, &preview.candidate, || panic!(
            "corrupt callback"
        ))
        .is_err());
    assert_eq!(files(dir.path()), before);
}

#[test]
fn checked_remove_deactivates_orphaned_dependencies_without_deleting_objects() {
    let dir = tempfile::tempdir().unwrap();
    let shared = source(dir.path(), "shared", &[]);
    let a = source(dir.path(), "a", &[("shared", "1.0.0")]);
    let p = project(dir.path());
    p.install_with_dependencies(&[a], &[shared]).unwrap();
    let preview = p.preview_remove("a").unwrap();
    assert_eq!(preview.previous.packages.len(), 2);
    assert!(preview.candidate.packages.is_empty());
    let objects = p.root().join(".orr/packages/objects");
    let before = files(&objects);
    assert_eq!(
        p.remove_checked("a", &preview.previous, &preview.candidate, || Ok(()))
            .unwrap(),
        preview.candidate
    );
    assert_eq!(files(&objects), before);
}
