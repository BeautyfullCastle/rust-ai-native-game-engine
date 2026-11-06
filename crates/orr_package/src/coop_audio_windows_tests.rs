use super::*;
use std::fs;
fn fixture(path: &Path) {
    let (manifest, objects) = super::super::tests::fixture();
    fs::write(
        path.join("bank.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    for (name, bytes) in objects {
        fs::write(path.join(name), bytes).unwrap();
    }
}
fn directory() -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("bank");
    fs::create_dir(&path).unwrap();
    fixture(&path);
    (root, path)
}
fn junction(path: &Path, target: &Path) {
    let output = std::process::Command::new("cmd.exe")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(path)
        .arg(target)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn windows_bank_matches_shared_pure_admission_without_extra_files_or_writes() {
    let (_root, path) = directory();
    let before: BTreeMap<_, _> = fs::read_dir(&path)
        .unwrap()
        .map(|e| {
            let path = e.unwrap().path();
            (
                path.file_name().unwrap().to_owned(),
                fs::read(path).unwrap(),
            )
        })
        .collect();
    fs::create_dir(path.join("unrelated")).unwrap();
    let loaded = load(&path).unwrap();
    let (manifest, objects) = super::super::tests::fixture();
    let expected = admit(&serde_json::to_vec(&manifest).unwrap(), &objects).unwrap();
    for (a, b) in loaded.records().iter().zip(expected.records()) {
        assert_eq!(
            (a.cue(), a.frames(), a.pcm_bytes()),
            (b.cue(), b.frames(), b.pcm_bytes())
        );
    }
    assert_eq!(loaded.records().len(), CUES.len());
    assert_eq!(fs::read_dir(&path).unwrap().count(), 12);
    for (name, bytes) in before {
        assert_eq!(fs::read(path.join(name)).unwrap(), bytes);
    }
}
#[test]
fn windows_bank_rejects_ambiguous_paths_before_opening_inputs() {
    for path in [
        "relative",
        "C:relative",
        "C:\\",
        "\\\\server\\share\\bank",
        "\\\\?\\C:\\bank",
        "C:\\a\\..\\b",
        "C:\\a\\.\\b",
        "C:\\a.",
        "C:\\a ",
        "C:\\NUL.txt",
        "C:\\CON .txt",
        "C:\\COM1",
        "C:\\LPT¹",
        "C:\\a:stream",
        "C:/bank",
        "C:\\a\\",
        "C:\\a\\\\b",
        "C:\\a\0",
    ] {
        assert!(input_parts(Path::new(path)).is_err(), "{path}");
    }
    assert!(input_parts(Path::new(&format!("C:\\{}", "a".repeat(240)))).is_err());
}
#[test]
fn windows_bank_oversize_missing_and_nonregular_manifest_or_pcm_fail_closed() {
    let (_root, path) = directory();
    for (name, max) in [("bank.json", 128 * 1024), ("death.pcm", 96008)] {
        let target = path.join(name);
        let original = fs::read(&target).unwrap();
        let oversized = vec![0; max + 1];
        fs::write(&target, &oversized).unwrap();
        assert!(load(&path).is_err());
        assert_eq!(fs::read(&target).unwrap(), oversized);
        fs::remove_file(&target).unwrap();
        assert!(load(&path).is_err());
        fs::create_dir(&target).unwrap();
        assert!(load(&path).is_err());
        fs::remove_dir(&target).unwrap();
        fs::write(target, original).unwrap();
    }
    assert!(load(&path).is_ok());
}
#[test]
fn windows_bank_rejects_junction_ancestry_manifest_and_pcm() {
    let (root, path) = directory();
    let alias = root.path().join("alias");
    junction(&alias, &path);
    assert!(load(&alias).is_err());
    fs::remove_dir(alias).unwrap();
    for name in ["bank.json", "death.pcm"] {
        let target = path.join(name);
        let saved = root.path().join("saved");
        fs::rename(&target, &saved).unwrap();
        junction(&target, root.path());
        assert!(load(&path).is_err());
        fs::remove_dir(&target).unwrap();
        fs::rename(&saved, &target).unwrap();
    }
}
#[test]
fn windows_bank_rejects_hardlinked_manifest_and_pcm_preserving_external_bytes() {
    let (root, path) = directory();
    for name in ["bank.json", "death.pcm"] {
        let target = path.join(name);
        let saved = root.path().join("external");
        fs::rename(&target, &saved).unwrap();
        let before = fs::read(&saved).unwrap();
        fs::hard_link(&saved, &target).unwrap();
        assert!(load(&path).is_err());
        assert_eq!(fs::read(&saved).unwrap(), before);
        fs::remove_file(&target).unwrap();
        fs::rename(&saved, &target).unwrap();
    }
}
#[test]
fn windows_bank_read_handles_block_writes_renames_and_directory_replacement() {
    let (root, path) = directory();
    let renamed = root.path().join("renamed");
    let mut read_names = Vec::new();
    load_with(&path, |name| {
        read_names.push(name.to_owned());
        assert!(fs::rename(&path, &renamed).is_err());
        for name in &read_names {
            let target = path.join(name);
            assert!(OpenOptions::new().write(true).open(&target).is_err());
            assert!(fs::rename(&target, root.path().join("moved-input")).is_err());
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(read_names.len(), 11);
    // Every reservation is released once admission returns.
    OpenOptions::new()
        .write(true)
        .open(path.join("bank.json"))
        .unwrap();
    fs::rename(&path, &renamed).unwrap();
}
#[test]
fn windows_bank_busy_input_fails_without_partial_publication_or_lingering_handles() {
    let (_root, path) = directory();
    for name in ["bank.json", "death.pcm"] {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(path.join(name))
            .unwrap();
        assert!(load(&path).is_err());
        drop(file);
        assert!(load(&path).is_ok());
    }
}
#[test]
fn windows_bank_corrupt_late_pcm_and_manifest_directed_paths_are_rejected() {
    let (_root, path) = directory();
    let pcm = path.join("death.pcm");
    let mut bytes = fs::read(&pcm).unwrap();
    bytes[8] ^= 1;
    fs::write(&pcm, &bytes).unwrap();
    assert!(load(&path).is_err());
    assert_eq!(fs::read(&pcm).unwrap(), bytes);
    fixture(&path);
    let (mut manifest, _) = super::super::tests::fixture();
    manifest["cues"][9]["file"] = "..\\outside.pcm".into();
    fs::write(
        path.join("bank.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    assert!(load(&path).is_err());
}
#[test]
fn windows_bank_late_unread_input_change_cannot_mix_with_pinned_manifest() {
    let (_root, path) = directory();
    assert!(load_with(&path, |name| {
        if name == "bank.json" {
            fs::write(path.join("death.pcm"), [0; 12]).unwrap();
        }
        Ok(())
    })
    .is_err());
    fixture(&path);
    assert!(load(&path).is_ok());
}
#[test]
fn windows_bank_accepts_readonly_assets_without_modifying_them() {
    let (_root, path) = directory();
    let files: Vec<_> = fs::read_dir(&path)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    let before: Vec<_> = files.iter().map(|p| fs::read(p).unwrap()).collect();
    for file in &files {
        let mut permissions = fs::metadata(file).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(file, permissions).unwrap();
    }
    let result = load(&path);
    for file in &files {
        let mut permissions = fs::metadata(file).unwrap().permissions();
        permissions.set_readonly(false);
        fs::set_permissions(file, permissions).unwrap();
    }
    assert_eq!(result.unwrap().records().len(), 10);
    for (file, bytes) in files.iter().zip(before) {
        assert_eq!(fs::read(file).unwrap(), bytes);
    }
}
