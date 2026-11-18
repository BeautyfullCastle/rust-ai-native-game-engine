use super::*;

fn directory() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

/// A directory junction `link` that points to `target`. It needs no special privilege.
fn junction(link: &Path, target: &Path) {
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .unwrap();
    assert!(status.status.success(), "mklink failed: {}", String::from_utf8_lossy(&status.stdout));
}

const HOSTILE: &str = "run-00000000000000000000000000000000-00000000-0000000000000000.json";

#[test]
fn repeated_save_is_immutable_and_bounded() {
    let dir = directory();
    let store = RunStore::open(dir.path(), &[]).unwrap();
    let a = store.save(b"first").unwrap();
    let b = store.save(b"second").unwrap();
    assert_ne!(a, b);
    assert_eq!(store.read(&a).unwrap(), b"first");
    assert_eq!(store.read(&b).unwrap(), b"second");
    assert_eq!(store.list().unwrap().len(), 2);
    assert!(store.save(&vec![0; MAX_SAVE_BYTES + 1]).is_err());
    assert_eq!(store.read(&a).unwrap(), b"first");
    assert!(store.read("../outside").is_err());
    // The staged file is gone after a publication: only the two generations are left.
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn rejects_junction_components_unsafe_paths_and_forbidden_roots() {
    let outer = directory();
    let private = outer.path().join("private");
    std::fs::create_dir(&private).unwrap();
    junction(&outer.path().join("link"), &private);
    let nested = private.join("nested");
    std::fs::create_dir(&nested).unwrap();
    assert!(RunStore::open(&outer.path().join("link").join("nested"), &[]).is_err(), "a junction in the middle");
    assert!(RunStore::open(&outer.path().join("link"), &[]).is_err(), "a junction as the folder");
    assert!(RunStore::open(&nested, &[outer.path().to_path_buf()]).is_err(), "inside a forbidden root");
    // Case does not hide a forbidden root.
    let upper = PathBuf::from(outer.path().to_string_lossy().to_uppercase());
    assert!(RunStore::open(&nested, &[upper]).is_err());
    assert!(RunStore::open(Path::new("relative\\saves"), &[]).is_err());
    assert!(RunStore::open(&nested.join("..").join("nested"), &[]).is_err());
    assert!(RunStore::open(&outer.path().join("missing"), &[]).is_err(), "the folder is not created");
    assert!(!outer.path().join("missing").exists());
    // A plain nested folder is accepted.
    RunStore::open(&nested, &[]).unwrap();
}

#[test]
fn hostile_generation_never_followed_and_old_save_preserved() {
    let dir = directory();
    let elsewhere = directory();
    let store = RunStore::open(dir.path(), &[]).unwrap();
    let a = store.save(b"safe").unwrap();
    junction(&dir.path().join(HOSTILE), elsewhere.path());
    assert!(store.read(HOSTILE).is_err());
    assert!(store.list().is_err());
    assert!(store.save(b"new").is_err());
    assert_eq!(store.read(&a).unwrap(), b"safe");
    assert_eq!(std::fs::read_dir(elsewhere.path()).unwrap().count(), 0, "nothing was written through the junction");
    std::fs::remove_dir(dir.path().join(HOSTILE)).unwrap();
    // A folder with the name of a generation is not a save either.
    std::fs::create_dir(dir.path().join(HOSTILE)).unwrap();
    assert!(store.read(HOSTILE).is_err());
    assert!(store.list().is_err());
}

#[test]
fn foreign_entries_are_preserved_and_scan_is_bounded() {
    let dir = directory();
    let store = RunStore::open(dir.path(), &[]).unwrap();
    for n in 0..=MAX_DIRECTORY_ENTRIES {
        std::fs::write(dir.path().join(format!("foreign-{n}")), b"untouched").unwrap();
    }
    assert!(store.list().is_err());
    assert!(store.save(b"new").is_err());
    assert_eq!(std::fs::read(dir.path().join("foreign-0")).unwrap(), b"untouched");
}

#[test]
fn capacity_boundary_allows_last_generation_then_refuses() {
    let dir = directory();
    let store = RunStore::open(dir.path(), &[]).unwrap();
    for n in 0..MAX_DIRECTORY_ENTRIES - 1 {
        std::fs::write(dir.path().join(format!("foreign-{n}")), b"keep").unwrap();
    }
    let last = store.save(b"last").unwrap();
    assert_eq!(store.list().unwrap(), vec![last.clone()]);
    assert!(store.save(b"over limit").is_err());
    assert_eq!(store.read(&last).unwrap(), b"last");
}

#[test]
fn rejects_linked_or_oversized_generation_without_losing_previous() {
    let dir = directory();
    let store = RunStore::open(dir.path(), &[]).unwrap();
    let safe = store.save(b"safe").unwrap();
    let other = store.save(b"other").unwrap();
    let other_path = dir.path().join(&other);
    std::fs::hard_link(&other_path, dir.path().join("foreign-link")).unwrap();
    assert!(store.read(&other).is_err(), "a second hard link");
    std::fs::remove_file(dir.path().join("foreign-link")).unwrap();
    assert_eq!(store.read(&other).unwrap(), b"other");
    OpenOptions::new().write(true).open(&other_path).unwrap().set_len((MAX_SAVE_BYTES + 1) as u64).unwrap();
    assert!(store.read(&other).is_err(), "more than 4 MiB");
    assert!(store.save(b"new").is_err());
    assert_eq!(store.read(&safe).unwrap(), b"safe");
}

#[test]
fn a_replaced_folder_is_refused_and_nothing_is_written_through_the_replacement() {
    let outer = directory();
    let elsewhere = directory();
    let path = outer.path().join("saves");
    std::fs::create_dir(&path).unwrap();
    let store = RunStore::open(&path, &[]).unwrap();
    let before = store.save(b"before").unwrap();
    std::fs::rename(&path, outer.path().join("original")).unwrap();
    junction(&path, elsewhere.path());
    assert!(store.save(b"pinned").is_err());
    assert!(store.list().is_err());
    assert!(store.read(&before).is_err());
    assert_eq!(std::fs::read_dir(elsewhere.path()).unwrap().count(), 0);
    assert_eq!(std::fs::read(outer.path().join("original").join(&before)).unwrap(), b"before");
    assert_eq!(std::fs::read_dir(outer.path().join("original")).unwrap().count(), 1);
}

#[test]
fn a_stage_cannot_be_renamed_or_deleted_while_it_is_written() {
    let dir = directory();
    let stage = dir.path().join(".pending-test");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(&stage)
        .unwrap();
    assert!(std::fs::rename(&stage, dir.path().join("moved")).is_err());
    assert!(std::fs::remove_file(&stage).is_err());
    drop(file);
    std::fs::remove_file(&stage).unwrap();
}

#[test]
fn name_and_path_rules() {
    assert!(inside(Path::new(r"C:\Data\Saves"), Path::new(r"c:\data")));
    assert!(!inside(Path::new(r"C:\Database"), Path::new(r"C:\Data")));
    assert!(validate_path(Path::new(r"C:\saves\runs")).is_ok());
    for bad in [r"C:\", r"saves", r"C:saves", r"\saves", r"C:\a\..\b", r"C:\a\b.", r"C:\a\b ", r"C:\a\b|c", r"C:\a\b:stream", r"\\server\share\saves"] {
        assert!(validate_path(Path::new(bad)).is_err(), "{bad}");
    }
    let default = default_directory().unwrap();
    assert!(default.ends_with(r"orrery\games\stormglass\runs"), "{}", default.display());
}
