use orr_package::{Project, Runtime};
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
fn restores_root_and_orphaned_dependency_without_original_sources() {
    let dir = tempfile::tempdir().unwrap();
    let dep = source(dir.path(), "dep", &[]);
    let art = source(dir.path(), "art", &[("dep", "1.0.0")]);
    let p = project(dir.path());
    let restore = p
        .install_with_dependencies(std::slice::from_ref(&art), std::slice::from_ref(&dep))
        .unwrap();
    let expected = p.remove("art").unwrap();
    fs::rename(art, dir.path().join("hidden-art")).unwrap();
    fs::rename(dep, dir.path().join("hidden-dep")).unwrap();
    let before = files(dir.path());
    let preview = p
        .preview_restore_removed("art", &expected, &restore)
        .unwrap();
    assert_eq!(preview.previous, expected);
    assert_eq!(preview.candidate, restore);
    assert_eq!(files(dir.path()), before);
    assert_eq!(
        p.restore_removed_checked("art", &expected, &restore, || Ok(()))
            .unwrap(),
        restore
    );
    assert!(!p.verify().unwrap().direct.contains_key("dep"));
    let mut after = files(dir.path());
    after.insert(
        PathBuf::from("project").join(LOCK),
        before[&PathBuf::from("project").join(LOCK)].clone(),
    );
    assert_eq!(after, before);
}

#[test]
fn restores_shared_dependency_and_retained_transitive_root_flags_exactly() {
    for remove in ["a", "dep"] {
        let dir = tempfile::tempdir().unwrap();
        let dep = source(dir.path(), "dep", &[]);
        let a = source(dir.path(), "a", &[("dep", "1.0.0")]);
        let b = source(dir.path(), "b", &[("dep", "1.0.0")]);
        let p = project(dir.path());
        let restore = p.install(&[dep, a, b]).unwrap();
        let expected = p.remove(remove).unwrap();
        assert!(expected.packages.contains_key("dep"));
        assert_eq!(
            p.preview_restore_removed(remove, &expected, &restore)
                .unwrap()
                .candidate,
            restore
        );
        assert_eq!(
            p.restore_removed_checked(remove, &expected, &restore, || Ok(()))
                .unwrap(),
            restore
        );
    }
}

#[test]
fn stale_active_lock_cannot_be_overwritten_by_old_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let a = source(dir.path(), "a", &[]);
    let b = source(dir.path(), "b", &[]);
    let p = project(dir.path());
    let restore = p.install(&[a]).unwrap();
    let expected = p.remove("a").unwrap();
    p.install(&[b]).unwrap();
    let before = files(dir.path());
    assert!(p.preview_restore_removed("a", &expected, &restore).is_err());
    assert!(p
        .restore_removed_checked("a", &expected, &restore, || panic!("stale callback"))
        .is_err());
    assert_eq!(files(dir.path()), before);
}

#[test]
fn rejects_non_inverse_and_forged_identity_before_callback() {
    let dir = tempfile::tempdir().unwrap();
    let a = source(dir.path(), "a", &[]);
    let p = project(dir.path());
    let restore = p.install(&[a]).unwrap();
    let expected = p.remove("a").unwrap();
    let before = files(dir.path());
    let mut forged = restore.clone();
    forged.packages.get_mut("a").unwrap().digest = "../../outside".into();
    for (name, current, target) in [
        ("a", &restore, &restore),
        ("missing", &expected, &restore),
        ("a", &expected, &forged),
    ] {
        assert!(p.preview_restore_removed(name, current, target).is_err());
        assert!(p
            .restore_removed_checked(name, current, target, || panic!("invalid callback"))
            .is_err());
    }
    assert_eq!(files(dir.path()), before);
}

#[test]
fn missing_or_tampered_cache_refuses_preview_and_commit() {
    for missing in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let a = source(dir.path(), "a", &[]);
        let p = project(dir.path());
        let restore = p.install(&[a]).unwrap();
        let expected = p.remove("a").unwrap();
        let object = p
            .root()
            .join(".orr/packages/objects")
            .join(&restore.packages["a"].digest);
        if missing {
            fs::rename(&object, dir.path().join("hidden-object")).unwrap();
        } else {
            fs::write(object.join("asset.txt"), "tampered").unwrap();
        }
        let before = files(dir.path());
        assert!(p.preview_restore_removed("a", &expected, &restore).is_err());
        assert!(p
            .restore_removed_checked("a", &expected, &restore, || panic!("cache callback"))
            .is_err());
        assert_eq!(files(dir.path()), before);
    }
}

#[test]
fn callback_failure_and_competing_writer_preserve_current_lock() {
    let dir = tempfile::tempdir().unwrap();
    let a = source(dir.path(), "a", &[]);
    let p = project(dir.path());
    let restore = p.install(&[a]).unwrap();
    let expected = p.remove("a").unwrap();
    let before = files(dir.path());
    let error = p
        .restore_removed_checked("a", &expected, &restore, || {
            assert_eq!(p.verify().unwrap(), expected);
            assert!(p
                .restore_removed_checked("a", &expected, &restore, || panic!("nested writer"))
                .is_err());
            Err("Room document changed".into())
        })
        .unwrap_err();
    assert!(error.to_string().contains("Room document changed"));
    assert_eq!(files(dir.path()), before);
    assert_eq!(
        p.restore_removed_checked("a", &expected, &restore, || Ok(()))
            .unwrap(),
        restore
    );
}

#[test]
fn existing_writer_is_retained_and_preview_remains_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let a = source(dir.path(), "a", &[]);
    let p = project(dir.path());
    let restore = p.install(&[a]).unwrap();
    let expected = p.remove("a").unwrap();
    fs::write(
        p.root().join(".orr/packages/writer.lock"),
        "existing writer",
    )
    .unwrap();
    let before = files(dir.path());
    assert!(p.preview_restore_removed("a", &expected, &restore).is_ok());
    assert!(p
        .restore_removed_checked("a", &expected, &restore, || panic!("writer callback"))
        .is_err());
    assert_eq!(files(dir.path()), before);
}
