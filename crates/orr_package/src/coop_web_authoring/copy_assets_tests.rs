use super::*;
use std::{fs, os::unix::fs::symlink};
struct Fixture {
    temp: tempfile::TempDir,
    source: PathBuf,
    output: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let output = temp.path().join("build");
        fs::create_dir(&source).unwrap();
        for name in super::super::offline::asset_names()
            .iter()
            .filter(|n| **n != "game.wasm")
        {
            let path = source.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, format!("bytes:{name}")).unwrap();
        }
        fs::create_dir_all(source.join("assets/nested/empty")).unwrap();
        fs::write(source.join("assets/nested/LICENSE.txt"), b"license").unwrap();
        Self {
            temp,
            source,
            output,
        }
    }
    fn options(&self) -> Options {
        Options {
            source: self.source.clone(),
            output: self.output.clone(),
            wasm: None,
        }
    }
    fn run(&self) -> Result<(), String> {
        copy(self.options(), |_, _| Ok(()))
    }
}
#[test]
fn repeat_copy_overwrites_only_intended_bytes_and_retains_empty_directories() {
    let f = Fixture::new();
    fs::create_dir(&f.output).unwrap();
    fs::write(f.output.join("game.wasm"), b"built").unwrap();
    fs::write(
        f.output.join("stormglass-foundry-offline.html"),
        b"delivery",
    )
    .unwrap();
    fs::create_dir(f.output.join("assets")).unwrap();
    fs::write(f.output.join("assets/unrelated.png"), b"extra").unwrap();
    f.run().unwrap();
    assert!(f.output.join("assets/nested/empty").is_dir());
    fs::write(f.source.join("game.js"), b"updated").unwrap();
    f.run().unwrap();
    assert_eq!(fs::read(f.output.join("game.js")).unwrap(), b"updated");
    assert_eq!(fs::read(f.output.join("game.wasm")).unwrap(), b"built");
    assert_eq!(
        fs::read(f.output.join("stormglass-foundry-offline.html")).unwrap(),
        b"delivery"
    );
    assert_eq!(
        fs::read(f.output.join("assets/unrelated.png")).unwrap(),
        b"extra"
    );
    assert_eq!(
        fs::read(f.output.join("assets/nested/LICENSE.txt")).unwrap(),
        b"license"
    );
}
#[test]
fn all_source_inputs_are_admitted_before_output_creation_or_replacement() {
    let f = Fixture::new();
    fs::remove_file(f.source.join("style.css")).unwrap();
    assert!(f.run().is_err());
    assert!(!f.output.exists());
    fs::write(f.source.join("style.css"), b"css").unwrap();
    fs::create_dir(&f.output).unwrap();
    fs::write(f.output.join("game.js"), b"old").unwrap();
    symlink("/missing", f.source.join("assets/nested/z-last")).unwrap();
    assert!(f.run().is_err());
    assert_eq!(fs::read(f.output.join("game.js")).unwrap(), b"old");
    assert_eq!(fs::read_dir(&f.output).unwrap().count(), 1);
}
#[test]
fn late_invalid_destination_is_rejected_before_any_write() {
    let f = Fixture::new();
    fs::create_dir(&f.output).unwrap();
    fs::write(f.output.join("game.js"), b"old").unwrap();
    fs::create_dir(f.output.join("style.css")).unwrap();
    assert!(f.run().is_err());
    assert_eq!(fs::read(f.output.join("game.js")).unwrap(), b"old");
    assert!(!f.output.join("assets").exists());
    fs::remove_dir(f.output.join("style.css")).unwrap();
    fs::write(f.output.join("assets"), b"not a dir").unwrap();
    assert!(f.run().is_err());
    assert_eq!(fs::read(f.output.join("assets")).unwrap(), b"not a dir");
}
#[test]
fn overlaps_symlink_ancestors_and_wasm_output_sources_are_refused() {
    let f = Fixture::new();
    let mut options = f.options();
    options.output = f.source.join("build");
    assert!(copy(options, |_, _| Ok(())).is_err());
    let mut options = f.options();
    options.output = f.temp.path().into();
    assert!(copy(options, |_, _| Ok(())).is_err());
    let alias = f.temp.path().join("alias");
    symlink(&f.source, &alias).unwrap();
    let mut options = f.options();
    options.source = alias;
    assert!(copy(options, |_, _| Ok(())).is_err());
    let foreign = f.temp.path().join("foreign");
    fs::create_dir(&foreign).unwrap();
    symlink(&foreign, &f.output).unwrap();
    assert!(f.run().is_err());
    assert_eq!(fs::read_dir(&foreign).unwrap().count(), 0);
    fs::remove_file(&f.output).unwrap();
    fs::create_dir(&f.output).unwrap();
    let wasm = f.output.join("game.wasm");
    fs::write(&wasm, b"old").unwrap();
    let mut options = f.options();
    options.wasm = Some(wasm);
    assert!(copy(options, |_, _| Ok(())).is_err());
}
#[test]
fn explicit_wasm_copies_exact_bytes_and_rejects_source_or_destination_symlinks() {
    let f = Fixture::new();
    let wasm = f.temp.path().join("compiled.wasm");
    fs::write(&wasm, b"trusted compiled bytes").unwrap();
    let mut options = f.options();
    options.wasm = Some(wasm.clone());
    copy(options, |_, _| Ok(())).unwrap();
    assert_eq!(
        fs::read(f.output.join("game.wasm")).unwrap(),
        b"trusted compiled bytes"
    );
    let alias = f.temp.path().join("wasm-link");
    symlink(&wasm, &alias).unwrap();
    let mut options = f.options();
    options.wasm = Some(alias);
    assert!(copy(options, |_, _| Ok(())).is_err());
    fs::remove_file(f.output.join("game.wasm")).unwrap();
    symlink(&wasm, f.output.join("game.wasm")).unwrap();
    let mut options = f.options();
    options.wasm = Some(wasm.clone());
    assert!(copy(options, |_, _| Ok(())).is_err());
    assert_eq!(fs::read(wasm).unwrap(), b"trusted compiled bytes");
}
#[test]
fn injected_midbatch_failure_reports_partial_publication_and_preserves_foreign_files() {
    let f = Fixture::new();
    fs::create_dir(&f.output).unwrap();
    fs::write(f.output.join("foreign"), b"untouched").unwrap();
    let mut calls = 0;
    let failure = copy(f.options(), |point, _| {
        if point == "before-replace" {
            calls += 1;
            if calls == 2 {
                return Err("injected second-file failure".into());
            }
        }
        Ok(())
    })
    .unwrap_err();
    assert!(failure.contains("after 1 file publications"), "{failure}");
    assert!(failure.contains("partial batch"));
    assert_eq!(fs::read(f.output.join("foreign")).unwrap(), b"untouched");
    let entries: Vec<_> = fs::read_dir(f.output.join("assets"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(entries
        .iter()
        .any(|n| n.to_string_lossy().starts_with(".orr-web-copy-")));
}
#[test]
fn changed_destination_after_preflight_is_never_replaced() {
    let f = Fixture::new();
    let failure = copy(f.options(), |point, path| {
        if point == "before-replace" {
            fs::write(path, b"foreign race winner").unwrap();
        }
        Ok(())
    })
    .unwrap_err();
    assert!(failure.contains("destination changed since batch preflight"));
    assert_eq!(
        fs::read(f.output.join("assets/characters.json")).unwrap(),
        b"foreign race winner"
    );
}
#[test]
fn oversized_input_and_deep_assets_fail_before_any_output() {
    let f = Fixture::new();
    let file = fs::OpenOptions::new()
        .write(true)
        .open(f.source.join("game.js"))
        .unwrap();
    file.set_len((MAX_FILE + 1) as u64).unwrap();
    assert!(f.run().is_err());
    assert!(!f.output.exists());
    file.set_len(0).unwrap();
    let mut nested = f.source.join("assets");
    for _ in 0..MAX_DEPTH {
        nested.push("d");
    }
    fs::create_dir_all(nested).unwrap();
    assert!(f.run().is_err());
    assert!(!f.output.exists());
}

#[test]
fn existing_regular_target_changed_after_preflight_is_preserved() {
    let f = Fixture::new();
    fs::create_dir_all(f.output.join("assets")).unwrap();
    let target = f.output.join("assets/characters.json");
    fs::write(&target, b"original intended replacement").unwrap();
    let failure = copy(f.options(), |point, path| {
        if point == "before-replace" {
            assert_eq!(path, target);
            // Same pathname remains a regular file, but its admitted inode and
            // metadata are no longer those captured by batch preflight.
            fs::remove_file(path).unwrap();
            fs::write(path, b"foreign replacement must survive").unwrap();
        }
        Ok(())
    })
    .unwrap_err();
    assert!(
        failure.contains("destination changed since batch preflight"),
        "{failure}"
    );
    assert!(failure.contains("after 0 file publications"), "{failure}");
    assert_eq!(
        fs::read(&target).unwrap(),
        b"foreign replacement must survive"
    );
    assert!(!f.output.join("assets/enemies.png").exists());
}

#[test]
fn failure_after_replace_reports_visible_uncertain_batch_without_rollback() {
    let f = Fixture::new();
    fs::create_dir_all(f.output.join("assets")).unwrap();
    let target = f.output.join("assets/characters.json");
    fs::write(&target, b"old build bytes").unwrap();
    fs::write(f.output.join("foreign"), b"unrelated sentinel").unwrap();
    let failure = copy(f.options(), |point, path| {
        if point == "after-replace" {
            assert_eq!(path, target);
            return Err("injected failure after visible replacement".into());
        }
        Ok(())
    })
    .unwrap_err();
    assert!(failure.contains("after 1 file publications"), "{failure}");
    assert!(failure.contains("partial batch"), "{failure}");
    assert!(failure.contains("durability may be uncertain"), "{failure}");
    assert_eq!(
        fs::read(&target).unwrap(),
        fs::read(f.source.join("assets/characters.json")).unwrap()
    );
    assert_eq!(
        fs::read(f.output.join("foreign")).unwrap(),
        b"unrelated sentinel"
    );
    assert!(!f.output.join("assets/enemies.png").exists());
}
