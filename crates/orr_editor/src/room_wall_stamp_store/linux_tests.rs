//! Linux filesystem contracts. No global hooks, sleeps, or timing-dependent races.
use super::*;
use serde_json::json;
use std::os::unix::{fs::symlink, net::UnixListener};

fn template(name: &str) -> Template {
    Template::capture(vec![(
        name.into(),
        json!({
            "RoomEscapeV1::Actor":{"kind":4,"ordinal":0},
            "orr_physics3d::Body":{
                "pos":[2,0.5,0],"rot":{"kind":"unit","x":0,"y":0,"z":0,"w":1},
                "vel":[0,0,0],"omega":[0,0,0],"inv_mass":0,"inv_inertia":[0,0,0],
                "linear_damping":0,"angular_damping":0,"kind":"static"
            },
            "orr_physics3d::Collider":{
                "shape":{"kind":"box","half_extents":[0.25,1,4]},
                "restitution":0,"friction":0.5,"layer":1,"mask":0
            }
        }),
    )])
    .unwrap()
}

fn path(dir: &Path) -> PathBuf {
    dir.join("walls 東京.room-stamp.json")
}

fn fifo(path: &Path) {
    assert!(std::process::Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("Linux mkfifo fixture")
        .success());
}

#[test]
fn regular_roundtrip_explicit_replacement_and_stale_digest() {
    let dir = tempfile::tempdir().unwrap();
    let path = path(dir.path());
    assert!(load(&path).unwrap().is_none());
    let original = template("first");
    let hash = save(&path, &original, None).unwrap();
    assert_eq!(
        load(&path).unwrap().unwrap(),
        (original.clone(), hash.clone())
    );
    assert!(save(&path, &template("other"), None).is_err());
    assert_eq!(fs::read(&path).unwrap(), original.encode().unwrap());
    let replacement = template("other");
    let next = save(&path, &replacement, Some(&hash)).unwrap();
    assert_ne!(hash, next);
    assert_eq!(load(&path).unwrap().unwrap(), (replacement, next));
    assert!(save(&path, &original, Some(&hash)).is_err());
    fs::remove_file(&path).unwrap();
    assert!(save(&path, &original, Some(&hash)).is_err());
}

#[test]
fn malformed_digest_never_changes_destination() {
    let dir = tempfile::tempdir().unwrap();
    let path = path(dir.path());
    let value = template("first");
    save(&path, &value, None).unwrap();
    let before = fs::read(&path).unwrap();
    for digest in [
        "".into(),
        "a".repeat(63),
        "a".repeat(65),
        "A".repeat(64),
        "g".repeat(64),
    ] {
        assert!(save(&path, &template("other"), Some(&digest)).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}

#[test]
fn portable_names_absolute_paths_and_real_ancestors_required() {
    let dir = tempfile::tempdir().unwrap();
    assert!(checked_path(Path::new("relative.room-stamp.json")).is_err());
    for name in [
        "NUL.room-stamp.json",
        "COM¹.room-stamp.json",
        "bad?.room-stamp.json",
        "bad.json",
    ] {
        assert!(checked_path(&dir.path().join(name)).is_err());
    }
    assert!(checked_path(&dir.path().join("..").join("a.room-stamp.json")).is_err());
    assert!(checked_path(&dir.path().join("missing").join("a.room-stamp.json")).is_err());
    fs::write(dir.path().join("file"), b"file").unwrap();
    assert!(checked_path(&dir.path().join("file").join("a.room-stamp.json")).is_err());
    fs::create_dir(dir.path().join("real")).unwrap();
    symlink(dir.path().join("real"), dir.path().join("link")).unwrap();
    assert!(checked_path(&dir.path().join("link").join("a.room-stamp.json")).is_err());
    symlink(dir.path().join("absent"), dir.path().join("dangling")).unwrap();
    assert!(checked_path(&dir.path().join("dangling").join("a.room-stamp.json")).is_err());
}

#[test]
fn symlink_and_dangling_file_rejected_without_touching_target() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target");
    let path = path(dir.path());
    fs::write(&target, b"unchanged").unwrap();
    symlink(&target, &path).unwrap();
    assert!(load(&path).is_err());
    assert!(save(&path, &template("first"), None).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"unchanged");
    fs::remove_file(&target).unwrap();
    assert!(load(&path).is_err());
    assert!(save(&path, &template("first"), None).is_err());
    assert!(!target.exists());
}

#[test]
fn malformed_oversized_directory_and_fifo_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = path(dir.path());
    for bytes in [b"{\"unexpected\":true}".to_vec(), vec![b' '; MAX_BYTES + 1]] {
        fs::write(&path, bytes).unwrap();
        assert!(load(&path).is_err());
    }
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(load(&path).is_err());
    assert!(save(&path, &template("first"), None).is_err());
    fs::remove_dir(&path).unwrap();
    fifo(&path);
    assert!(load(&path).is_err());
    assert!(save(&path, &template("first"), None).is_err());
}

#[test]
fn unix_socket_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = path(dir.path());
    let _socket = UnixListener::bind(&path).expect("mandatory Unix socket fixture; no skip");
    assert!(load(&path).is_err());
    assert!(save(&path, &template("first"), None).is_err());
}

#[test]
fn same_length_inode_substitution_before_open_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = path(dir.path());
    fs::write(&path, b"first").unwrap();
    let checked = checked_path(&path).unwrap();
    let substitute = dir.path().join("substitute");
    fs::write(&substitute, b"other").unwrap();
    assert_ne!(
        fs::metadata(&path).unwrap().ino(),
        fs::metadata(&substitute).unwrap().ino()
    );
    let error =
        read_optional_with_hooks(&checked, || fs::rename(&substitute, &path).unwrap(), || {})
            .unwrap_err();
    assert!(error.contains("changed while opening"), "{error}");
}

#[test]
fn same_length_inode_substitution_after_read_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = path(dir.path());
    fs::write(&path, b"first").unwrap();
    let checked = checked_path(&path).unwrap();
    let substitute = dir.path().join("substitute");
    fs::write(&substitute, b"other").unwrap();
    assert!(
        read_optional_with_hooks(&checked, || {}, || fs::rename(&substitute, &path).unwrap())
            .is_err()
    );
}

fn mutate_same_inode(path: &Path) {
    let inode = fs::metadata(path).unwrap().ino();
    fs::write(path, b"other").unwrap();
    // Explicit timestamp makes this deterministic even on coarse timestamp filesystems.
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(12345))
        .unwrap();
    assert_eq!(fs::metadata(path).unwrap().ino(), inode);
}

#[test]
fn same_inode_same_length_mutation_before_open_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = path(dir.path());
    fs::write(&path, b"first").unwrap();
    let checked = checked_path(&path).unwrap();
    assert!(read_optional_with_hooks(&checked, || mutate_same_inode(&path), || {}).is_err());
}

#[test]
fn same_inode_same_length_mutation_after_read_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = path(dir.path());
    fs::write(&path, b"first").unwrap();
    let checked = checked_path(&path).unwrap();
    let error = read_optional_with_hooks(&checked, || {}, || mutate_same_inode(&path)).unwrap_err();
    assert!(error.contains("changed while reading"), "{error}");
}

#[test]
fn fifo_substitution_before_open_returns_without_writer() {
    let dir = tempfile::tempdir().unwrap();
    let path = path(dir.path());
    fs::write(&path, b"first").unwrap();
    let checked = checked_path(&path).unwrap();
    let error = read_optional_with_hooks(
        &checked,
        || {
            fs::remove_file(&path).unwrap();
            fifo(&path);
        },
        || {},
    )
    .unwrap_err();
    assert!(error.contains("changed while opening"), "{error}");
}

#[test]
fn symlink_substitution_before_open_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = path(dir.path());
    fs::write(&path, b"first").unwrap();
    let target = dir.path().join("target");
    fs::write(&target, b"other").unwrap();
    let checked = checked_path(&path).unwrap();
    assert!(read_optional_with_hooks(
        &checked,
        || {
            fs::remove_file(&path).unwrap();
            symlink(&target, &path).unwrap();
        },
        || {}
    )
    .is_err());
    assert_eq!(fs::read(&target).unwrap(), b"other");
}

#[test]
fn hardlinks_are_accepted_and_replacement_detaches_only_destination() {
    let dir = tempfile::tempdir().unwrap();
    let path = path(dir.path());
    let original = template("first");
    let hash = save(&path, &original, None).unwrap();
    let alias = dir.path().join("alias.room-stamp.json");
    fs::hard_link(&path, &alias).unwrap();
    assert_eq!(
        load(&alias).unwrap().unwrap(),
        (original.clone(), hash.clone())
    );
    let replacement = template("other");
    save(&path, &replacement, Some(&hash)).unwrap();
    assert_eq!(load(&alias).unwrap().unwrap().0, original);
    assert_eq!(load(&path).unwrap().unwrap().0, replacement);
    assert_ne!(
        fs::metadata(&path).unwrap().ino(),
        fs::metadata(&alias).unwrap().ino()
    );
}

#[test]
fn create_publication_race_does_not_overwrite_new_destination() {
    let dir = tempfile::tempdir().unwrap();
    let path = path(dir.path());
    assert!(
        save_with_before_publish(&path, &template("first"), None, || {
            fs::write(&path, b"concurrent writer").unwrap();
        })
        .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), b"concurrent writer");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn replacement_final_check_to_persist_gap_is_not_compare_and_swap() {
    let dir = tempfile::tempdir().unwrap();
    let path = path(dir.path());
    let hash = save(&path, &template("first"), None).unwrap();
    let replacement = template("other");
    // This is the documented trusted-directory limitation, not hostile-race protection.
    save_with_before_publish(&path, &replacement, Some(&hash), || {
        fs::write(&path, b"writer after final check").unwrap();
    })
    .unwrap();
    assert_eq!(load(&path).unwrap().unwrap().0, replacement);
}

#[test]
fn directory_inode_substitution_rejected_by_opened_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("parent");
    fs::create_dir(&path).unwrap();
    let observed = fs::symlink_metadata(&path).unwrap();
    fs::rename(&path, dir.path().join("old-parent")).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(verify_directory(&path, &observed).is_err());
}

#[test]
fn growth_after_read_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = path(dir.path());
    fs::write(&path, b"first").unwrap();
    let checked = checked_path(&path).unwrap();
    assert!(read_optional_with_hooks(
        &checked,
        || {},
        || {
            fs::write(&path, vec![b' '; MAX_BYTES + 1]).unwrap();
        }
    )
    .is_err());
}
