use super::*;

fn prefab_path(directory: &Path, filename: &str) -> PathBuf {
    directory.join(filename)
}

#[test]
fn save_and_load_preserve_bytes_and_return_lowercase_sha256() {
    let directory = tempfile::tempdir().unwrap();
    let path = prefab_path(directory.path(), "Wood.mesh-prop-prefab.json");
    let contents = b"opaque prefab bytes\0with binary data";

    let saved_digest = save_new(&path, contents).unwrap();
    let (loaded, loaded_digest) = load(&path).unwrap();

    assert_eq!(loaded, contents);
    assert_eq!(saved_digest, loaded_digest);
    assert_eq!(saved_digest.len(), 64);
    assert!(saved_digest
        .bytes()
        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()));
}

#[test]
fn save_new_never_replaces_an_existing_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = prefab_path(directory.path(), "Existing.mesh-prop-prefab.json");
    fs::write(&path, b"original").unwrap();

    assert!(save_new(&path, b"replacement").is_err());
    assert_eq!(fs::read(path).unwrap(), b"original");
}

#[test]
fn save_and_load_enforce_the_codec_byte_limit() {
    let directory = tempfile::tempdir().unwrap();
    let path = prefab_path(directory.path(), "Large.mesh-prop-prefab.json");
    let oversized = vec![b'x'; MAX_BYTES + 1];

    assert!(save_new(&path, &oversized).is_err());
    fs::write(&path, &oversized).unwrap();
    assert!(load(&path).is_err());
}

#[test]
fn filename_validation_rejects_nonportable_or_wrong_suffixes() {
    for filename in [
        "mesh-prop-prefab.json",
        "../Escape.mesh-prop-prefab.json",
        "CON.mesh-prop-prefab.json",
        "NUL.txt.mesh-prop-prefab.json",
        "CONIN$.mesh-prop-prefab.json",
        "CONOUT$.txt.mesh-prop-prefab.json",
        "Name:stream.mesh-prop-prefab.json",
        "Name*.mesh-prop-prefab.json",
        "Name.mesh-prop-prefab.json.",
    ] {
        assert!(
            validate_filename(filename).is_err(),
            "accepted {filename:?}"
        );
    }
}

#[test]
fn absolute_path_validation_rejects_unc_and_device_prefixes() {
    let directory = tempfile::tempdir().unwrap();
    let unc = Path::new("//server/share/Asset.mesh-prop-prefab.json");
    assert!(checked_path(unc).is_err());

    let device_path = Path::new(r"\\.\C:\Asset.mesh-prop-prefab.json");
    assert!(checked_path(device_path).is_err());

    let valid_parent = directory.path();
    assert!(checked_path(&prefab_path(valid_parent, "Asset.mesh-prop-prefab.json")).is_ok());
}

#[cfg(unix)]
#[test]
fn parent_symlinks_are_rejected() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let real_parent = directory.path().join("real");
    let linked_parent = directory.path().join("linked");
    fs::create_dir(&real_parent).unwrap();
    symlink(&real_parent, &linked_parent).unwrap();

    assert!(checked_path(&prefab_path(&linked_parent, "Asset.mesh-prop-prefab.json")).is_err());
}

#[cfg(unix)]
#[test]
fn file_symlinks_are_rejected_for_load_and_save() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let original = directory.path().join("original.json");
    let linked = prefab_path(directory.path(), "Linked.mesh-prop-prefab.json");
    fs::write(&original, b"content").unwrap();
    symlink(&original, &linked).unwrap();

    assert!(load(&linked).is_err());
    assert!(save_new(&linked, b"replacement").is_err());
    assert_eq!(fs::read(original).unwrap(), b"content");
}
